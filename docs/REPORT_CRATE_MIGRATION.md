# Report Crate Migration Checklist

This document tracks the migration of report-related functionality from the main `stutter` crate into the `stutter-report` workspace crate.

## Migration Status

- [ ] `stutter-report/src/lib.rs`: Remove "placeholder" / "future migration" warnings
- [x] `stutter-report/src/model/`: Move pure report model structs from main crate
- [ ] `stutter-report/src/load.rs`: Implement real file loading and return `ReportModel`
- [ ] `stutter-report/src/render/text/`: Split text rendering into smaller modules (header, quality, cluster, correlation, frame, diagnosis)
- [ ] `stutter-report/src/diff/mod.rs`: Implement diff logic based on real report model
- [ ] `stutter-report/src/load.rs`: Delete scaffold `unsupported_operation`
- [ ] `stutter-report/src/analysis/mod.rs`: Migrate report analysis logic

## Acceptance Criteria

- No behavior change in CLI report rendering.
- `render_report()` output remains byte-for-byte identical.
- `stutter report diff` uses `stutter-report` logic.
- Main crate compiles through re-exports.
