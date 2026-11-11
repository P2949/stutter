# E16: status explanation review: AlreadySatisfied for the decision fields, one operator gap documented (2026-11-11)

Question (PLAN.md E16): can the existing status flow show why a legal game affinity/cgroup-placement candidate was or was not selected, and whether
telemetry is only application present-call observation? Inspected: `stutter autotune-status --json` of the installed binary (read-only, run with the
service's HOME), the status model/renderers and their tests, and the new read-only `xtask service-explain` (E7).

## Existing output already answers the decision questions (AlreadySatisfied, exact tests)
`AutotuneStatus` carries phase, mode, target, focus group, score, active profile and candidate, kept actions, last decision, rollback availability and
path, cooldown, a structured `planner` summary, `data_quality`, `last_fault`, the manual restore command and the history path. Existing tests:
`status_from_daemon_state_reports_snapshot_fields`, `status_from_daemon_state_includes_top_denied_reason` (why a candidate was denied),
`status_from_daemon_state_includes_structured_planner_summary`, `status_from_daemon_state_renders_daemon_lifecycle_labels_and_candidates`,
`status_from_daemon_state_lists_all_profile_memory_kept_actions`, `empty_history_reports_disabled_status`, `rollback_event_clears_active_profile_and_rollback_available`,
`restored_event_clears_rollback_active_profile_and_last_fault`, `status_from_history_matches_text_example` (text) and `json_status_serializes` (JSON), and
`load_autotune_status_prefers_daemon_state_snapshot_when_present`. Unknown or absent values render as `null` or an explicit label rather than a fake value.
Live (installed service, `HOME=/var/lib/stutter/autotune-p2949`): phase `cooldown`, mode `ApplyLowRisk`, no target, no kept action, no fault, last decision
"restored: startup crash recovery rollback succeeded; affected_tasks=0".

## What the status does not carry, and where it is answered instead (no shipped-code change)
Owner confidence (suspected vs confirmed), the running binary's identity and the authority boundary are not status fields. They are answered by `xtask service-explain`
(E7, 6 tests): owner hints are only ever `ExternalSuspected` (process-name hints, not task-level proof; the ScxLoader hint's kernel-thread false positive is
caveated), the configured binary hash for both units equals the NixosConf pin `6922c7c6...` (the running image is unverified: `/proc/<pid>/exe` is unreadable),
and the legal operations are the production `AUTONOMOUS_CONTRACTS` (`CpuAffinityProfile`, `CgroupPlacement`) with `HIGH_RISK_APPLY_IMPLEMENTED = false`.
Telemetry provenance: `data_quality` is `null` with no target, reported as Unknown; the application source stage (`application_present_submit`, not scan-out)
is recorded in the W21/E6 evidence and the layer, not in this status.

## Operator gap (documented, not changed)
`stutter autotune-status` reads the invoking user's `~/.local/state/stutter/autotune/`. Run as the desktop user it prints `Disabled` / "no autotune history found"
while the installed service is active, because the service keeps its state under its own HOME. Run with the unit's HOME it is correct. Changing the shipped
crate (a serialized, agent-exposed struct) for this would need its own review and, for a runtime-code change, a guarded activation; it is out of scope for an
xtask-only change. `service-explain --instance <name>` is the supported read-only way to see the service's state.
