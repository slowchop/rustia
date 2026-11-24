# Browser Correctness Runner

## Overview

Pinned Playwright WebKit support for the shared WebAssembly correctness suite. Headless WebKit execution is distinct from the manual real-Safari release check.

## Interface

- [package.json](./package.json) and [package-lock.json](./package-lock.json) pin the private runner's Playwright dependency.
- [webkit-runner.cjs](./webkit-runner.cjs) serves as Cargo's Wasm test runner: it starts `wasm-bindgen-test-runner`, opens its local test page in headless WebKit, emits results and shared golden transcripts, and checks the result.
- [result.cjs](./result.cjs) exports `requirePassed`; [result.test.cjs](./result.test.cjs) covers its acceptance and rejection cases.
- [web-live-example.cjs](./web-live-example.cjs) serves the documented application with Python on an ephemeral loopback port, then checks initialization, returned `42`, console output, repeated calls and reload in real headless WebKit. It can save an optional screenshot through `RUSTIA_EXAMPLE_SCREENSHOT`.
- [The parent coordinator](../run-browser-correctness.sh) selects browser/mode suites and validates aggregate execution evidence.
- [record-race.cjs](./record-race.cjs) compares native and WebKit/Wasm race traces, drives the [recording canvas](./race-frame.html) by tick, and atomically replaces the MP4 after validating its frames.
- [record-edits.cjs](./record-edits.cjs) drives the editor in `--hot-reload` mode. It drives real key events, draws a blinking caret at the text insertion point, shows keycaps and checks every tick through valid and invalid edits. It captures frames at 1280×720.
- [render-demo-gif.sh](./render-demo-gif.sh) exports a recorded MP4 as a looping GIF and replaces the output only after encoding succeeds.
- [render-hot-reload-gif.sh](./render-hot-reload-gif.sh) converts the hot-reload MP4 to a looping GIF.

## Invariants

- A successful result requires exactly one successful test summary with at least one passing test and zero failures. A launched browser alone is not execution evidence.
- The test server binds to loopback on an ephemeral port. Server startup, browser launch, navigation, and test completion have bounded waits.
- Browser and server cleanup runs even when execution or result validation fails.

## Side Effects

The runner starts a local test server and headless browser, executes guest correctness tests, and writes test output and diagnostics. It terminates its server during cleanup, escalating to forced termination after five seconds if needed. It does not collect performance measurements.

## Verification

From the repository root: `node --test tools/browser/result.test.cjs` validates result parsing without launching a browser; `python3 tools/check-browser-correctness.py` validates workflow/runner policy. Full browser execution follows [the platform contract](../../design/96-browser-platform-acceptance.md). After building the [browser recipe](../../demos/web-live/README.md), `node tools/browser/web-live-example.cjs` checks the generated application without collecting timings.
