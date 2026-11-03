# Public source readiness

Date: 2026-10-27 (Australia/Hobart).
Audited baseline: `a62ef708e63c02fdea5630c32e3f8c78d173b82a`.
Verdict: do not change repository visibility yet.

## Decision and scope

The user authorized local preparation and an audit before making source public.
The user did not authorize a visibility change, publication, a public post, or a
package release. All workspace packages remain at 0.0.1 with publication disabled.
Source access is separate from supported 0.1/1.0 releases. Requirements in
[90](90-release-readiness.md), [91](91-semver-policy.md),
[92](92-first-release.md) and [00](00-constraints.md) remain in force for releases.

Public source permits inspection without implying a stable API, full Luau
parity, support for platforms without execution evidence, an invitation to
contribute, or guaranteed support. Outside contribution and issue policies are
undecided; source visibility does not require opening either channel.
The initial documentation remains README, rustdoc and CONTRIBUTING; a website,
new execution tier or additional API layer is not needed to open the source.

## Consumer results checked

The other chat completed its validation and stopped editing. Its logs, local
commit identities and saved grids were inspected directly for this audit.

| Consumer | Merged-runtime result | Delivery state |
| --- | --- | --- |
| WOTS | 453 regular tests plus 15 full-campaign correctness tests pass; strict lint and application build pass | Local commit `73012eb2dd96f3819db0bdfcb64eb6faeb0e35b0`; not pushed |
| civfun | 62 host tests, lint and CLI builds pass; all three candidate grids are byte-identical to the previous runtime | Local commit `5b5956ca62bc769ad51bae476f3e7bf06cc4f4ad`; not pushed |
| Grabby | Actual vendor refresh and application acceptance are unfinished | Existing vendor pin remains; required draft-PR approval was rejected by automatic approval review in the owning chat |

WOTS and civfun select the audited Rustia baseline above. civfun's Washington
and Attila outputs match all 4,160 saved plots; Aztec retains its existing
3,842/4,160 result. Matching the previous runtime does not fix that unrelated
residual mismatch. Grabby has not passed against this merged runtime and must
not be reported as complete. This source-access task does not authorize retrying
the rejected Grabby publication action.

## Audit scope and method

- The checkout is not shallow. The local scan covered 1,367 commits, 11,437
  unique blobs and seven annotated tags reachable through local refs; tree
  objects were inventoried too. Tracked baseline content has 1,716 entries.
- A live read-only remote inventory advertised 139 refs, including pull-request
  refs and tags. Their objects were available locally. An additional 14 objects
  outside local-ref reachability were scanned separately; no branch or tag was
  deleted or changed.
- Custom rules inspected file paths, blob contents, commit/tag contents,
  credential markers, infrastructure addresses, local paths and private-repo
  indicator matches. They read objects directly, including removed files.
- Official Gitleaks 8.30.1 was downloaded into the local audit directory and its
  archive checksum verified against the release checksums. Its default rules
  scanned Git history with all local refs plus all advertised remote tips,
  recursive decoding and archive inspection enabled. Inline suppression comments
  and repository ignore files were not used to hide findings. The resulting
  report contains zero secret findings; custom-rule matches were reviewed too.
- Read-only GitHub inspection covered repository visibility/settings, runner-group
  public-access policy, 75 issue/PR descriptions, four issue comments, zero inline
  review comments, the release list and the Actions artifact/run inventories.
  A retained CI log archive was sampled for infrastructure metadata.

These checks do not prove the absence of every secret. Full review of all retained
Actions logs, artifact contents, PR review-summary bodies, attachments and any
other externally stored content remains a visibility gate. Repository history
does not contain the contents of the separate public upstream submodule history;
the tracked gitlink and URL are included in the audit.

Detailed values, file/object locations, raw settings responses and logs are kept
outside this public-bound repository. Do not copy those values into public docs,
issues, reviews or commit messages.

## Findings

### Blocking: infrastructure metadata in current files and history

Fourteen current benchmark-evidence files contain a public IPv4 field and a cloud
instance ID: 28 infrastructure-identifying fields in total. They describe older
cloud benchmark instances; the audit does not assert that those instances are
still running. Older reachable history also retains infrastructure metadata,
including material removed from today's tree. A local home-directory path also
remains in recorded runtime-version evidence, with more local paths in older
research artifacts. One historical commit message contains an address-like value
that requires review as part of history remediation.

This violates the repository's public/private boundary even though the secret
scanner reports no credentials. Deleting files from the current tree does not
remove earlier versions, branches, tags, PR refs or hosted copies. Do not rewrite
history, delete evidence, alter measurement values or rehash acceptance evidence
merely to make the scan pass. Preserve the private original and choose a reviewed
publication strategy first.

Private-key markers found in removed cloud-coordinator code belong to validation
patterns and synthetic self-tests. Credential-bearing URL matches use secret
variable interpolation, not a committed literal credential. Those reviewed
matches are not verified leaked private keys or tokens. Local paths and
contributor identities need privacy review separately from credential scanning.

### Blocking: Actions history would expose infrastructure metadata

The run inventory contains 1,051 Actions runs and 12 retained artifacts. One
sampled CI archive has 73 runner/machine/local-path metadata matches across its
logs. The whole retained log/artifact set has not been cleared for publication.
GitHub explicitly states that Actions history and logs become visible when a
private repository is made public:
[visibility documentation](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/managing-repository-settings/setting-repository-visibility).

Review or privately retain and remove disallowed hosted material before any
visibility change. Removal is a separate action requiring an agreed retention
plan; no runs, logs or artifacts were deleted during this audit.

### Blocking: public CI needs a deliberate trust boundary

The relevant runner groups currently disallow public repositories. Changing
visibility without a CI migration would prevent existing jobs from using those
runners. Simply enabling public access would create a different problem:
ordinary pull-request workflows execute code on persistent self-hosted runners.
The benchmark workflow's fork guard does not establish isolation for all jobs,
and editable workflow conditions alone are not a sufficient runner trust boundary.

The maintainer selected private self-hosted correctness on 2026-11-01, with
ordinary generic runner groups and no hosted fallback. Keep the organization
groups private-only and block public repositories and fork PRs in every job.
Before public visibility, separately review an isolated execution model;
workflow conditions alone are not a runner trust boundary. Preserve required
checks and reviews while changing routing. The benchmark group remains separate.
GitHub describes the risks and private-only default in its
[runner access documentation](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/manage-access).
No runner access, Actions permissions or repository protections were changed.

### Remaining public-facing preparation

Issues, wiki and Pages are disabled; there are no GitHub releases. A reviewed
reporting route and a private vulnerability-reporting channel must exist before
inviting public reports. The fork-approval setting cannot be inspected through
its endpoint while this repository is private; do not claim it is configured.
Recheck actual settings and protections at the eventual visibility decision.

## Remediation order

1. Preserve the private repository, evidence and consumer commits. Decide whether
   to sanitize the existing repository/history and hosted surfaces or prepare a
   separate clean source export. A clean export avoids exposing old hosted logs,
   but requires deliberate treatment of history-dependent verification and
   benchmark provenance. It must not pretend to retain the original commit graph.
2. Prepare a reversible local sanitized candidate. Remove infrastructure identity
   from its public projection, preserve measurement values and attribution, and
   document how its public evidence relates to the private original. Review the
   consequences for commit-identity checks before selecting the final route.
3. Establish the public CI trust boundary and privately test the resulting check
   inventory without granting public PRs access to persistent private machines.
4. Finish or explicitly record the remaining Grabby consumer gate. Keep WOTS and
   civfun's local pin updates separate from any public-source action.
5. Finish the externally hosted-content review and reporting setup. Re-run secret,
   privacy, documentation/package and required correctness validation against the
   exact proposed public source and refs. Independently review the findings.
6. Present the exact source, history/ref strategy, hosted-content disposition and
   visibility action to the user for explicit approval. Keep packages unpublished.

No history rewrite, GitHub mutation, package-policy change, push or publication
was performed by this preparation work. This record reports a blocked visibility
decision, not a public-release acceptance.

## Local preparation verification

`just check-quick` passes, including 124 compiler unit tests. Separate metadata,
package-file, publication-policy, release-note, workflow-policy and generated
progress checks pass. Modified-document local links and `git diff --check` pass.
The private evidence archive was reopened and checked: both Gitleaks reports are
empty and the 28 confirmed infrastructure fields are preserved for remediation.
No Cargo manifest, lockfile, version or publication setting changed. Full release
preflight was not rerun for these documentation-only edits; it remains mandatory
before any push and does not replace the outstanding visibility gates above.

## Full-history projection rehearsal (2026-10-28)

The user selected a new repository containing the full cleaned history while the
existing repository remains private. This authorizes local preparation. No new
repository, visibility change, package publication, hosted deletion, or Actions
permission change has been performed by this rehearsal.

The frozen remote snapshot contains 1,004 commits and seven annotated tags. The
local projection preserves every commit, parent order, author, committer, date,
message and tag message. It redacts confirmed infrastructure identifiers and
private absolute paths. All commit IDs change; 879 original commit signatures
are removed. A rewritten commit is a derived record, not an originally signed
commit. Private original bundles and independent verification records are
retained separately. Reviewed synthetic address fixtures and author attribution
are preserved.

The new repository will not inherit the old repository's Actions logs, artifacts,
or pull-request records. Five archive branches retain seven commits that were
reachable only through old pull-request refs. Standard branches and tags retain
the complete cleaned graph; old pull-request refs are excluded from the new
container. The existing private repository and its hosted evidence stay intact.

[history-projection.json](history-projection.json) records the complete old-to-new
commit map, protected runtime-input fingerprints, and the original/projected
hashes of redacted metadata at the frozen head. The runtime crates, conformance
corpus, workload sources, dependency lockfile and toolchain are identical across
each mapped commit. Benchmark measurements and historical source IDs remain
recorded as originally captured. Legacy artifact digests bind the original
private bytes; they do not authenticate redacted provider metadata. Projected
metadata has separate hashes. This does not establish identical rebuilt binaries
or replace original CI acceptance.

The reachability check now refuses missing identifiers instead of silently
ignoring them. Historical source IDs resolve through the explicit projection;
all mapped commits and protected inputs are verified. Existing reachable tree
identifiers remain valid. This check is part of the full preflight inventory.

The explicit [public source profile](public-source-profile.json) selects private self-hosted
correctness jobs and bounded fuzz checks. It has no benchmark timing authority,
publication lane, repository write credentials, or benchmark-runner
access. The rendered benchmark page labels its data as historical; automatic
public measurement, history refresh and publication remain paused. The separate
[private maintainer bridge](#private-maintainer-cohort-bridge-status-2026-10-29)
is available for reviewed manual runtime comparisons only. Broader automation
still needs separate design and authorization; source access does not authorize
that expansion or public performance claims.

The user authorized the private staging repository and its Actions-disabled
initial transfer. The repository was verified private, empty and Actions-disabled
before the transfer attempt. The first-history bootstrap guard is retained in
staging commit `417828e` for provenance and is not part of the current source;
current pushes use the ordinary exact-HEAD pre-push guard. All 75 hosted refs
were compared with the reviewed inventory and a fresh clone was inspected.
CI correctness checks gate acceptance of staged revisions, with exact CI
evidence retained privately. The repository remains private and packages remain
unpublished; this is not supported-release acceptance or a visibility decision.

## Private maintainer cohort bridge status (2026-10-29)

A separate private maintainer bridge is available for manually dispatched,
reviewed exact-cohort runtime comparisons only. It does not add a timing
authority, runner, credential, or benchmark workflow to the public source
repository. Correctness workflows remain private-only, self-hosted and read-only. The
public coordinator continues to refuse benchmark commands, and automatic
per-commit/history measurement, landscape refresh, and publication remain
paused.

The bridge stores comparison output privately. It does not update public
history or authorize public claims. Comparisons require independent raw-data
review before acceptance. The profile state records this narrow bridge
availability, not a successful run, a current performance result, or a change
to source visibility or package publication.
