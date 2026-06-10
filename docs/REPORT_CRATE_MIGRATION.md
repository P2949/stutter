# Report Crate Migration

This document records the current ownership boundary between the main `stutter`
runtime crate and the `stutter-report` workspace crate.

The initial report-crate migration is complete. The stable report data model and
basic self-contained HTML rendering now live in `stutter-report`.

This means the initial migration checklist is closed. It does **not** mean every
report-related responsibility has moved out of the main `stutter` crate.

## Current ownership boundary

`stutter-report` owns:

- Stable report model types used for persisted/reportable output.
- Basic self-contained HTML rendering for report data that no longer depends on
  the runtime crate.
- Crate-local report helpers that are independent of live capture, CLI command
  wiring, and runtime-specific state.

The main `stutter` crate still owns:

- Rich CLI report assembly.
- Runtime report generation that depends on live capture state.
- Command-specific report orchestration.
- Report flows tied to profiler, tuner, validation, or daemon behavior.
- Integration tests and fixtures that exercise end-to-end command output.

## Remaining follow-up work

Future report work should move only clearly reusable, runtime-independent report
model or rendering code into `stutter-report`.

Do not move command orchestration, live runtime state handling, or tuning/report
flows just to make the migration look more complete. Those pieces still belong in
the main crate until they have a stable crate-independent boundary.

## Documentation rule

Documentation should describe the initial migration as complete, while still
making the remaining main-crate ownership explicit.

Avoid stale wording such as "the remaining main-crate report logic is tracked" if
it implies an unfinished placeholder migration. Prefer wording that says the
initial migration is complete and that some report responsibilities intentionally
remain in the main crate.
