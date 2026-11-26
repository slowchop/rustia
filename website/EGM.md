# Rustia website

## Overview

The static homepage and playground exercise the checked-out Rustia facade in
browser workers. The website is a standalone, unpublished Cargo workspace.

## Interface

- index.html, garden.mjs and garden-player.mjs implement the editable homepage.
  The homepage includes the roadmap and starts on narrow screens with the
  garden playing and the editor collapsed. The intro, feature list, and roadmap come from
  [project-copy.json](../project-copy.json) via `node tools/sync-project-copy.js`.
- site-header.html supplies the shared homepage and documentation navigation.
  rustia-mark.svg supplies the staggered dash mark for website headers and browser
  tabs. The build copies it to the site root. rustia-readme-mark.svg is the
  transparent, theme-aware mark used by the repository README.
- doc-index.html links to the first script, embedding guide, API reference, and
  garden walkthrough. The build publishes it at /docs/.
- garden-source.html documents the files that run and draw the garden. The
  build publishes it at /docs/examples/garden/ and keeps /garden-source/ available.
- [playground](./playground/EGM.md) owns projects, preview modes and the terminal UI.
- bridge.rs, garden.rs and playground.rs connect browser workers to the Rustia
  facade. Sprite output contains coordinates and a shape ID. Console input
  resumes a suspended Luau coroutine.
- source-editor.mjs and editor.mjs supply shared editing and file selection.
  analysis/ contains the pinned, licensed Luau browser analyzer.
- build.sh prepares the pinned Rust and Wasm tools and stages the static
  site in dist/, including the facade Rustdoc under /docs/api/.
  render-home-comparisons.py reads table labels, project and platform links,
  and evidence explanations from [the shared Rustdoc comparison](../crates/rustia/compare.md).
  It adds homepage icon styling and narrow-screen cards. The chart uses the
  [retained benchmark cohort](../bench/history/site-comparison.json) and links
  to the archived timings. The renderer
  also generates search and social descriptions from the homepage intro.
  _headers defines the deployed policy.

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
