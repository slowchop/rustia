# Fixed-work benchmark evidence

Accepted fixed-work comparisons live in a directory named for the full
candidate engine commit:

```text
<full-commit>/
  manifest.json
  <workload>-abba-A1.json
  <workload>-abba-B1.json
  ...
  analysis.json
  final-<workload>-1.json
  final-<workload>-2.json
  final-<workload>-3.json
```

The JSON files are unmodified `rustia.bench.raw-samples.v1` sidecars. The
manifest records baseline and candidate commits, source and binary SHA-256
values, toolchain and host provenance, commands, fixed work, repetitions, run
order, cooldown and exclusion rules, every leg summary, the comparison and
confidence-interval method, and the predeclared material-regression threshold.
`analysis.json` is the unmodified `rustia.bench.fixed-sample-analysis.v1`
output from `bench/analyze-fixed-samples.js`; the manifest records its command
and SHA-256 value. Accepted eight-leg comparisons use `--expected-legs 4` and
`--require-full-commits`. Their verdict is based on the conservative envelope
of pooled and leg-clustered hierarchical bootstrap intervals, with the
analyzer's nominal-coverage caveat retained in the output.

`../runs.jsonl` remains the append-only public summary schema. Evidence files do
not add fields to it, and a pooled or hand-computed result must never be inserted
as a history row. The `final-<workload>-*` rows must come from the exact candidate
binary and match the actual summaries appended by those three invocations.

Provider-backed runs may additionally retain `host.json`, `provider.json`,
`protocol.json`, `gate.jsonl`, `qualification.json`, qualification-only
sidecars, and a remote log. Those files are provenance around the same fixed
sample schema; qualification samples are never inputs to `analysis.json` or
rows in `../runs.jsonl`.

Evidence-only protocols that do not append throughput history live outside this
tree. The paired many-VM string-memory corpus is under
`../../evidence/vm-string-memory/<full-commit>/` and has its own sample and
analysis schemas.
