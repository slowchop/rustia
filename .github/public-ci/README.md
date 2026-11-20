# Public source correctness CI

The [active CI](../workflows/ci.yml) and [differential fuzz workflow](../workflows/error-fuzz.yml)
run on standard GitHub-hosted runners. The [selected profile](../../design/public-source-profile.json)
is `public-hosted`. GitHub reads the workflows under `.github/workflows/`. The
copies in this directory let the source-profile validator detect inventory drift.

| Trigger | Required execution |
| --- | --- |
| Repository becomes public (`public`) | Both workflows run the required inventory against the default-branch commit. |
| Pull request | Both workflows check the proposed change. Only repository collaborators can open PRs. |
| Push to `main` | CI runs the preflight, platform and browser checks. |
| Manual dispatch | CI can also exercise Windows native and real Firefox. The fuzz workflow can run a longer campaign. |

The required checks use standard Ubuntu and macOS VMs. They cover the preflight
stages and aggregate, eight target builds, native Linux and macOS, Node/V8,
headless WebKit, the browser embedding example and differential smoke. Windows
uses the standard hosted Windows VM on dispatch. All jobs have read-only source
access, pinned actions and checkout credentials disabled. Keep private runner
groups and benchmark infrastructure outside this repository. Native Intel Mac
and remote Safari execution are still open platform acceptance items.

Private Actions billing prevented the proposed hosted rehearsal from running.
At visibility change, inspect the actual job outcomes in both workflows for
the reviewed commit. The selected profile and static validators establish the
configuration. Hosted acceptance requires completed checks on the hosted VMs.
Run `node tools/check-workflows.js` to check the workflow policy. See
[the launch decision](../../design/public-source-activation.md).
