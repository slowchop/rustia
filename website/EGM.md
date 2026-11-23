# Rustia website

## Overview

The static homepage and playground exercise the checked-out Rustia facade in
browser workers. The website is a standalone, unpublished Cargo workspace.

## Interface

- index.html, garden.mjs and garden-player.mjs implement the editable homepage.
  The narrow homepage starts with the garden playing and the editor collapsed.
- garden-source.html documents the files that run and draw the garden. The
  build publishes it at /garden-source/.
- [playground](./playground/EGM.md) owns projects, preview modes and the terminal UI.
- bridge.rs, garden.rs and playground.rs connect browser workers to the Rustia
  facade. Sprite output contains coordinates and a shape ID. Console input
  resumes a suspended Luau coroutine.
- source-editor.mjs and editor.mjs supply shared editing and file selection.
  analysis/ contains the pinned, licensed Luau browser analyzer.
- build.sh prepares the pinned Rust and Wasm tools and stages the static
  site in dist/. render-home-comparisons.py renders the README comparison
  tables into the static homepage. _headers defines the deployed policy.

## Invariants

Only packaged assets enter the publish directory. Project modules are resolved
from project files. Guest code receives host capabilities through the Rustia
facade. Browser storage and page JavaScript are outside that interface.

## Side effects

Workers run on the client. Project edits and preferences write to browser
storage. The build writes generated assets to dist/ and compiler output to the
configured Cargo target directory.

## Verification

Run the build and the two Node checks in README.md. In a browser, verify the
homepage, an edited multi-file console project, an input prompt, and a sprite
preview.

Parent map: [Rustia](../EGM.md).
