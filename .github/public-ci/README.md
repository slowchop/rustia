# Proposed public correctness CI

These files are dormant preparation for issue #29. GitHub executes workflows
only from `.github/workflows/`; nothing here changes ordinary private routing,
runner access, repository visibility or package publication.

The proposed pair uses standard `ubuntu-24.04`, `macos-15` ARM64 and
`windows-2025` X64 hosted VMs. Public fork PRs execute the same required
correctness inventory with read-only repository permissions, pinned actions,
disabled caches, no retained checkout credentials and no private secrets.
The preflight stages, aggregate, eight build targets, native Linux/macOS,
Node/V8, headless WebKit, documented browser example and differential smoke
remain intact. Windows/real Firefox and longer fuzz campaigns remain manual.
The private desktop Safari route is explicitly omitted: #40 is deferred and no
hosted GUI execution evidence exists. Intel native acceptance #39 also remains
open. A build target is not execution acceptance.

`node tools/check-public-source-candidate.js` compares these files with a
complete, narrowly defined transformation of active private CI. It rejects
lost jobs/commands, changed runners, write credentials, skipped public PRs and
other drift. Its negative fixtures run through the existing workflow preflight.
`--write` refreshes proposals explicitly before validation; checks never write.

Standard hosted VMs have cold caches and limited disk/RAM. These proposals
have not executed on hosted runners. Private self-hosted green checks and
static validation are not hosted acceptance. A bounded private hosted rehearsal
must demonstrate the full required inventory before activation. Two manual
readiness workflows are bound to the maintainer, private repository, main ref,
exact SHA and dispatch confirmation. Non-required Windows/Firefox/campaign work
needs a separate `extended` opt-in, off for the initial rehearsal. Existing
billing limits are not changed. Preserve checks if disk or time fails; do not silently select paid
larger runners or weaken coverage.

Activation replaces the ordinary pair with these exact proposal bytes, removes
the private-only rehearsal pair and explicitly selects `correctness_ci:
public-hosted` in the source profile. An absent selector retains ordinary private
routing. This CI selection grants no visibility or benchmark authority. The
disposable activation test runs real validator/self-test entrypoints with the
selected hosted mode; benchmark provenance and paused publication stay intact.
See [the activation checklist](../../design/public-source-activation.md).

References: [standard runner specifications](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
and [self-hosted runner access](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/manage-access).
