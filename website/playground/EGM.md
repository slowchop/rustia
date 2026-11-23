# Playground

The playground page selects a local starter and reuses the site's shared editor
and sprite player. playground.mjs owns drafts, mode selection and program
worker lifetimes. worker.mjs keeps a suspended console program or a separate
persistent interactive console in its own worker. repl.mjs owns console input
and history. panes.mjs saves pane widths. The race page runs the site's separate
animated console example.

Program input and output are text. Project files and pane settings are
browser-local. Verify starter execution, module edits, browser draft recovery,
input resumption, and canvas mode switching in a browser.

Parent map: [website](../EGM.md).
