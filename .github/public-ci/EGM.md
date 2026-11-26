# Public correctness workflow inventory

Parent: [repository map](../../EGM.md).

- [ci.yml](ci.yml) and [error-fuzz.yml](error-fuzz.yml) mirror the hosted
  correctness inventory selected in the [source profile](../../design/public-source-profile.json).
  The differential job checks parser acceptance and compares compiler-case results against pinned Luau.
  GitHub executes the files in [workflows](../workflows/).
- [README.md](README.md) describes the triggers and execution scope and links the verified launch results.
- [SECURITY.md](SECURITY.md) points to the [active reporting policy](../../SECURITY.md).
- The active dispatch-only [Quick](../workflows/quick.yml),
  [Bulk](../workflows/bulk.yml) and
  [staged correctness](../workflows/staged-correctness.yml) workflows are kept
  outside this mirrored launch inventory. The public workflow-profile validator
  pins their inputs, run titles, hosted runners, read-only permissions, stage
  gates, protected policy provenance and exact-checkout verification order.

The source workflow validator checks the required job inventory, hosted runners,
read-only permissions and event triggers. The `public` event starts both
workflows at the visibility change. Timing authority stays private.
