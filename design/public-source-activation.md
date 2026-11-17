# Public-source activation proposal

Status: preparation only; no visibility, hosted-content deletion, runner-access,
repository-protection, package-publication or larger-runner approval is implied.
Issue [#29](https://github.com/slowchop/rustia/issues/29) owns work status.

## Proposed public contract

Publish source for inspection and experimental use under the existing
MIT OR Apache-2.0 licensing. Keep every package at 0.0.1 with `publish = false`.
Do not promise stable APIs, full Luau parity, unexecuted platforms, reliable
performance comparisons or support deadlines. Supported releases remain #34.

The proposed contribution policy accepts reproducible bug reports and focused
pull requests on a best-effort basis. Ask for discussion before large changes;
include tests and relevant design/oracle evidence. Maintainers may decline work
or close stale reports. This is the proposed default for approval, not an already
adopted invitation. Current CONTRIBUTING/README policies remain unchanged.

Use GitHub private vulnerability reporting for security reports. The proposed
root policy is [SECURITY.md](../.github/public-ci/SECURITY.md). GitHub documents
this feature for public repositories; the present private repository's unavailable
endpoint does not prove enablement. Enable and verify the route at the approved
visibility transition before inviting security reports. The policy includes a
contact-only public fallback that forbids publishing vulnerability details.

## Before requesting visibility approval

1. Review the exact candidate commit, every remotely advertised branch/tag/PR
   ref and the private original's preservation record. Repeat credential and
   privacy scans, classify synthetic/version/attribution matches explicitly,
   and verify license/notices and package-file checks. A clean current tree
   does not clear old history or hosted material.
2. Inventory every Actions run **and every attempt**, job metadata, logs,
   artifacts, check output/annotations, release assets and issue/PR/review bodies.
   Download, hash and reopen sensitive evidence privately before proposing any
   removal. Record unavailable content and exact removal IDs. Deleting only
   logs does not remove private runner identities from job metadata.
3. Independently review the [dormant hosted proposal](../.github/public-ci/README.md).
   Organization policies must independently deny all public repositories access
   to persistent private runners; editable workflow conditions are insufficient.
   Keep benchmark access separate. Recheck repository/organization secrets,
   variables, environments, Actions permissions and runner-group policy live.
4. Run the bounded manual standard-hosted readiness rehearsal for #29, after
   independent review and ordinary private CI pass. Bind it to the private
   repository, maintainer, main ref and exact reviewed SHA, with explicit dispatch
   confirmation and non-required extended work disabled. Preserve existing
   billing limits; do not enable paid larger runners or change billing settings.
   Execute the complete required public check
   inventory on its actual proposed revision, including cold-cache disk and
   platform bootstrap. Static tests/private self-hosted results are insufficient.
   This validates the user-requested preparation; it is not an automatic hosted
   fallback or performance run. Retain all failures and actual hosting evidence.
5. Reconcile exact consumer records. Source access is separate from consumer
   deployment and supported releases: keep unfinished Grabby acceptance explicit,
   and do not imply WOTS/civfun or a previous candidate establishes current rollout.
6. Present one concrete approval packet: reviewed source/ref inventory, CI
   activation diff and executed checks, proposed reporting/contribution policy,
   exact privately retained hosted-removal manifest, repository-setting changes
   and ordered visibility action. Keep raw infrastructure evidence private.

## Only after the corresponding explicit approvals

1. Establish a quiet transition window and verify that candidate refs and hosted
   inventories still match the approved packet. Stop on new/unreviewed changes.
2. Apply the reviewed CI activation changes, selected-profile/validator updates
   and any approved protections. Preserve required checks, merge commits and
   independent review. Current protections and auto-merge are not already enforced.
3. Privately retain and verify the final hosted delta, then remove only approved
   sensitive runs/artifacts. Recheck all surfaces; do not delete historic evidence
   or rewrite source history merely to satisfy a pattern scanner.
4. Perform the specifically approved visibility action. Confirm that the historic
   and operations repositories remain private and packages remain unpublished.
   Enable and verify the private reporting channel and adopt only the approved
   policy text. Recheck actual public CI, refs and hosted content after transition.
5. If any gate fails, stop publication/promotion and report the actual failure.
   Making the repository private again cannot recall public clones or forks.

The present private benchmark intake may deliberately refuse a public source
repository. Do not silently broaden that trust contract at visibility change;
continued private diagnostics need their own reviewed intake decision. Public
benchmark automation, history refresh and publication remain paused.

GitHub references: [visibility consequences](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/managing-repository-settings/setting-repository-visibility),
[private reporting](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/configure-vulnerability-reporting/configure-for-a-repository),
and [runner access](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/manage-access).
