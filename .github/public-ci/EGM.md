# Dormant public-source proposals

Parent: [repository map](../../EGM.md).

- `ci.yml` and `error-fuzz.yml` are inactive hosted workflow proposals derived
  from the current private correctness inventory. They use fixed standard hosted
  labels and no private credentials or runner groups.
- `README.md` explains verification, unproven hosted execution and activation.
- `SECURITY.md` is a proposed root security policy; it becomes operative only
  after maintainer approval and a verified private reporting route.

No file in this directory is an active GitHub Actions workflow. Current private
ordinary workflow and source-profile bytes remain unchanged. The separate manual
rehearsal pair in `.github/workflows/` tests this exact command inventory with
private/maintainer/main/SHA/confirmation guards and extended work off by default.
`tools/check-public-source-candidate.js` checks proposal and rehearsal parity,
negative fixtures and a disposable selected-profile activation without changing
the live source profile or visibility.
