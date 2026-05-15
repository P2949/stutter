# Changelog

Release notes use these daemon-safety categories:

- Safety
- Tuning behavior
- Rollback
- Config migration

## Unreleased

- Safety: added daemon service planning, watchdog status, and release readiness
  gates.
- Rollback: low-risk service templates keep restore hooks on stop.
