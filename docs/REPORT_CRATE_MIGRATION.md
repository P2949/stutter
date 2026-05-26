# Report Crate Migration Checklist

This document tracks the migration of report-related functionality from the main `stutter` crate into the `stutter-report` workspace crate.

## Migration Status

- [x] `stutter-report/src/lib.rs`: Remove "migration placeholder" comments and errors
- [x] `stutter-report/src/model/`: Move pure report model structs from main crate
- [x] `stutter-report/src/load.rs`: Implement real file loading and return `ReportModel`
- [x] `stutter-report/src/render/text/`: Split text rendering into smaller modules (header, quality, cluster, correlation, frame, diagnosis)
- [x] `stutter-report/src/diff/mod.rs`: Implement diff logic based on real report model

## Acceptance Criteria

- [x] No behavior change in CLI report rendering.
- [x] `render_report()` output remains byte-for-byte identical.
- [x] `stutter report diff` uses `stutter-report` logic.
- [x] Main crate compiles through re-exports.
