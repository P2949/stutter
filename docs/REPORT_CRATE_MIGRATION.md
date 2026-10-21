# Report Crate Migration

This document records the ownership boundary between the main `stutter` runtime
crate and the `stutter-report` workspace crate.

The initial report-crate migration is complete for the first stable boundary.
This means the initial migration checklist is closed. That does **not** mean every report-related responsibility has moved out of the
main `stutter` crate.

## Completed migration boundary

- [x] `stutter-report/src/lib.rs`: real crate API, no migration-placeholder errors.
- [x] `stutter-report/src/model/`: migrated report-domain model modules.
- [x] `stutter-report/src/load.rs`: crate-local report model loading.
- [x] `stutter-report/src/analysis/`: crate-local report model analysis.
- [x] `stutter-report/src/render/text/`: migrated text rendering boundary.
- [x] `stutter-report/src/render/html.rs`: basic self-contained HTML rendering for migrated report models.
- [x] `stutter-report/src/diff/mod.rs`: diff logic based on the migrated report model.
- [x] `stutter report diff`: uses `stutter-report` diff logic through main-crate integration.
- [x] Main crate compiles through compatibility re-exports where needed.

## Current ownership boundary

`stutter-report` owns:

- Stable report-domain model types used for persisted or reportable output.
- Crate-local JSON loading for `ReportModel`.
- Crate-local model analysis.
- Crate-local report diffing.
- Migrated text rendering.
- Basic self-contained HTML rendering for report data that no longer depends on
  runtime-specific state.

The main `stutter` crate still owns:

- Conversion from recorder/session artifacts into rich report input models.
- Full CLI report command integration.
- Rich interactive HTML report model assembly and template rendering.
- Report JSON output and compatibility model types.
- Regression summaries that still depend on main-crate artifact/session types.
- Golden-report compatibility tests that protect historical CLI output.
- Report flows tied to profiler, tuner, validation, or daemon behavior.
- Compatibility re-exports while downstream/main-crate call sites finish migrating.

## Remaining follow-up work

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
      validation-corpus tests.

Future report work should move only clearly reusable, runtime-independent report
model or rendering code into `stutter-report`. Do not move command orchestration,
live runtime state handling, or tuning/report flows merely to make the migration
look more complete.

## Acceptance criteria for the initial migration

- [x] No behavior change in CLI text report rendering.
- [x] `render_report()` output remains byte-for-byte identical for protected golden cases.
- [x] `stutter report diff` uses `stutter-report` diff logic.
- [x] Main crate compiles through re-exports and compatibility adapters.
- [x] `stutter-report` exposes crate-local load/analyze/diff/render tests.

## Documentation rule

Describe the initial migration as complete while keeping the remaining main-crate
ownership explicit. Avoid wording that implies either that the migration is
unfinished placeholder work or that all report code has already moved.
