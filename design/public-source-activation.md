# Experimental public source launch

## Decision of 2026-11-20

Release the cleaned `slowchop/rustia` source under MIT OR Apache-2.0 for
inspection and experimental use. The visibility change applies only to this
repository. The historic repository, operations repository and benchmark
service stay private. All eight workspace crates stay unpublished. Supported
releases and consumer deployment have separate acceptance gates.

Keep GitHub Issues enabled for the maintainer backlog. The public may read existing
issues and pull requests. Restrict creation of both to repository collaborators
on an ongoing basis. Verify both controls at the visibility change. Handle security
reports through [GitHub private vulnerability reporting](../SECURITY.md).
Enable and verify the reporting route after the repository becomes public.
Until the route works, reporters should keep details private.

The private rehearsal could not start its hosted jobs because of private
Actions billing. The workflows use the visibility transition for the first
required hosted run. Land both on the default branch before changing visibility. The GitHub `public` event triggers the standard
hosted correctness and differential-fuzz suites for the default-branch HEAD.
Keep existing billing limits and the complete required inventory. Keep
private runner groups and benchmark credentials inaccessible to public repos.

At transition, verify the reviewed refs and publicly visible repository
content, the repository and runner access settings, and the private status of the
historic and operations repositories. Record the two workflow run IDs, check
their actual jobs for the visibility commit, and confirm private reporting works.
Record failures or skipped required jobs as failures. Local preflight and any
earlier private jobs cannot establish hosted execution. If the launch run needs
repair, keep its result and verify the repaired revision on hosted CI.

[Hosted workflow inventory](../.github/public-ci/README.md) ·
[Readiness and history audit](98-public-source-readiness.md) ·
[Release readiness](90-release-readiness.md).

## Historical preparation for issue #29

The following checklists document the earlier proposal. It envisaged a
successful **private** hosted rehearsal before visibility and considered public
bug reports and focused external contributions. The decision above supersedes
the rehearsal sequence and contribution proposals. The privacy review and
private isolation requirements continue to apply.

### Original pre-approval checklist

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
3. Independently review the [hosted CI inventory](../.github/public-ci/README.md).
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

### Original transition checklist

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
