# Luau 0.709 Upstream Conformance Corpus

These files are copied verbatim from the upstream Luau repository:

- repository: <https://github.com/luau-lang/luau>
- commit: `b968ef742741bb2b703afc3b3c53f06608c87481`
- source directory: `tests/conformance/`
- license: MIT, copied in `LICENSE.txt`

The rustia manifest tracks these files as `pending-impl` until the interpreter
implements enough of each script to execute it under the conformance harness.
`pending-impl` rows are path-checked but not run.

`PROVENANCE.tsv` records one row per copied script. The `oracle_command` column
uses the upstream `Luau.Conformance` test case that owns the script at the
pinned commit.
