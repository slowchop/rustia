# Public correctness workflow inventory

Parent: [repository map](../../EGM.md).

- [ci.yml](ci.yml) and [error-fuzz.yml](error-fuzz.yml) mirror the hosted
  correctness inventory selected in the [source profile](../../design/public-source-profile.json).
  GitHub executes the files in [workflows](../workflows/).
- [README.md](README.md) describes the triggers and execution scope and links the verified launch results.
- [SECURITY.md](SECURITY.md) points to the [active reporting policy](../../SECURITY.md).

The source workflow validator checks the required job inventory, hosted runners,
read-only permissions and event triggers. The `public` event starts both
workflows at the visibility change. Timing authority stays private.
