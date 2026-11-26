# Experimental public source launch

## Launch on 20 November 2026

Rustia became public at [commit `e54fd4e`](https://github.com/slowchop/rustia/commit/e54fd4eb763837ab186f3a3fd05b75c76bcda551) after [PR #72](https://github.com/slowchop/rustia/pull/72).
The repository contains the experimental interpreter under MIT OR Apache-2.0.
All eight workspace packages had publication disabled at launch.
The historic and operations repositories stayed private.

At launch, the contribution policy let anyone read issues and pull requests
and reserved opening them for collaborators. See the
[current contribution policy](../CONTRIBUTING.md#contributing) and
[security policy](../SECURITY.md).

The private rehearsal stopped before GitHub assigned a runner. At launch, the
`public` event ran both workflows on the reviewed commit using standard GitHub-hosted
runners. All 15 required CI jobs and the differential smoke job passed. The eight
target builds checked compilation. The runtime jobs exercised Linux x86-64, macOS
arm64, Node/V8 and headless WebKit. Windows, real Firefox and the long fuzz
campaign were manual jobs and did not run at launch. [CI run](https://github.com/slowchop/rustia/actions/runs/44695193616) · [Differential smoke run](https://github.com/slowchop/rustia/actions/runs/44695193576).

The launch checks also verified anonymous access to the README and demos, both
README examples from the pinned Git dependency, and the exclusion of public
repositories from the private runner groups. Supported package release and
consumer acceptance are tracked separately. [Release checklist](90-release-readiness.md).

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
