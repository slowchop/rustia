# Public source correctness CI

The [active CI](../workflows/ci.yml) and [differential fuzz workflow](../workflows/error-fuzz.yml)
run on standard GitHub-hosted runners. The [selected profile](../../design/public-source-profile.json)
is `public-hosted`. GitHub reads the workflows under `.github/workflows/`. The
copies in this directory let the source-profile validator detect inventory drift.
The `state: private-rehearsal` field dates from the original rehearsal.
`correctness_ci` selects the active runners. `visibility_authorized: false` keeps
repository visibility changes outside the authority of this file.

| Trigger | Required execution |
| --- | --- |
| Repository becomes public (`public`) | Both workflows run the required inventory against the default-branch commit. |
| Pull request | Both workflows check the proposed change. Only repository collaborators can open PRs. |
| Push to `main` | CI runs the preflight, platform and browser checks. |
| Manual dispatch | CI can also exercise Windows native and real Firefox. The fuzz workflow can run a longer campaign. |

The separate [bulk trust fixture](../workflows/bulk.yml) is dispatch-only. It
accepts the controller's four typed inputs, uses them in the run title and
verifies the exact candidate checkout in the `integrity` job. It uses a standard
hosted runner and read-only permissions. It does not activate the controller or
replace the current correctness workflows.

The required checks use standard Ubuntu and macOS VMs. They cover the preflight
stages and aggregate, eight target builds, native Linux and macOS, Node/V8,
headless WebKit, the browser embedding example and differential smoke. Windows
uses the standard hosted Windows VM on dispatch. All jobs have read-only source
access, pinned actions and checkout credentials disabled. Keep private runner
groups and benchmark infrastructure outside this repository. Native Intel Mac
and remote Safari execution are still open platform acceptance items.

The public launch completed all 16 required jobs on `e54fd4e`. Run
`node tools/check-workflows.js` to verify the configuration locally.
The [launch record](../../design/public-source-activation.md) links both runs.
