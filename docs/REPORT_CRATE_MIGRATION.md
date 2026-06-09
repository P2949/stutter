# Report Crate Migration

This document tracks the report-code ownership boundary between the main
`stutter` runtime crate and the `stutter-report` workspace crate.

## Current status

The initial report-crate migration is complete for the first stable boundary:

- [x] `stutter-report/src/lib.rs`: real crate API, no migration-placeholder errors
- [x] `stutter-report/src/model/`: migrated report-domain model modules
- [x] `stutter-report/src/load.rs`: crate-local report model loading
- [x] `stutter-report/src/analysis/`: crate-local report model analysis
- [x] `stutter-report/src/render/text/`: migrated text rendering boundary
- [x] `stutter-report/src/render/html.rs`: basic self-contained HTML rendering for migrated report models
- [x] `stutter-report/src/diff/mod.rs`: diff logic based on the migrated report model
- [x] `stutter report diff`: uses `stutter-report` diff logic through main-crate integration
- [x] Main crate compiles through compatibility re-exports where needed

This means the initial migration checklist is closed. It does **not** mean every
report-related responsibility has moved out of the main `stutter` crate.

## Current ownership split

`stutter-report` owns:

- migrated report-domain model structs;
- crate-local JSON loading for `ReportModel`;
- crate-local model analysis;
- crate-local report diffing;
- migrated text rendering;
- basic self-contained HTML rendering for migrated report models.

The main `stutter` crate still owns:

- conversion from recorder/session artifacts into rich report input models;
- full CLI report command integration;
- rich interactive HTML report model assembly and template rendering;
- report JSON output and compatibility model types;
- regression summaries that still depend on main-crate artifact/session types;
- golden-report compatibility tests that protect historical CLI output;
- compatibility re-exports while downstream/main-crate call sites finish migrating.

## Remaining follow-up work

Track follow-up work here instead of treating the initial checklist as evidence
that all report work is done:

- [ ] Move richer artifact-to-report-input conversion behind a narrower report boundary.
- [ ] Decide whether the rich interactive HTML report pipeline should move into
      `stutter-report` or remain owned by the main crate as a CLI/report-output
      integration layer.
- [ ] Continue moving stable report-facing model fields into `stutter-report`
      when they no longer depend on main-crate runtime/session types.
- [ ] Keep typed ID migrations in migrated report models aligned with
      `stutter-core`.
- [ ] Preserve byte-for-byte text report compatibility unless a deliberate
      report-format migration is documented.
- [ ] Keep old artifact readability and degraded-evidence behavior covered by
      validation corpus tests.

## Acceptance criteria for the initial migration

- [x] No behavior change in CLI text report rendering.
- [x] `render_report()` output remains byte-for-byte identical for protected golden cases.
- [x] `stutter report diff` uses `stutter-report` diff logic.
- [x] Main crate compiles through re-exports and compatibility adapters.
- [x] `stutter-report` exposes crate-local load/analyze/diff/render tests.

## Documentation rule

Do not describe the report migration as “all report code is migrated” while the
main `stutter` crate still owns rich CLI report assembly, report JSON,
compatibility APIs, and the interactive HTML pipeline. Use “initial migration
complete” or “migrated boundary complete” for the current status.
