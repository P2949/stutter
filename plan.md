PROPOSAL 1: Replace `.unwrap()` in sysfs-reading production paths with `?`-propagated `anyhow::Result`
STATUS: COMPLETED 2026-05-16 — validated production paths in the listed files no longer contain the panic sites described here; remaining `.unwrap()` calls are test-only.
PRIORITY: CRITICAL
Justifies: a panic in `emergency_restore`, `startup_recovery`, or `system_context` during an already-degraded system state terminates the daemon without completing rollback.

CURRENT STATE:
`autotune/emergency_restore.rs` line ~1003: `write_controller_journal_clean(input.journal_path.as_deref().unwrap()).unwrap()` — called unconditionally before `restore_known_autotune_actions`. If the journal path is absent or the write fails, the daemon panics before restoring any actions.

`autotune/system_context.rs`: 19 `.unwrap()` calls on sysfs string parse operations (CPU frequency reads, IRQ affinity reads, GPU power reads). Each is a potential panic if sysfs returns unexpected content, which happens on kernel upgrades, driver changes, or when a device is removed mid-read.

`autotune/active_config.rs`: 32 `.unwrap()` calls; many are on `serde_json::to_value(&snapshot).unwrap()` in production snapshot paths (lines that are NOT inside `#[cfg(test)]`). JSON serialization of a known-valid struct is actually infallible here, but the call site does not document this invariant.

`autotune/startup_recovery.rs`: 47 `.unwrap()` calls; the function `check_and_recover_on_startup` calls journal reading and action restoration — both are I/O operations that can fail legitimately.

PROPOSED CHANGE:
In `emergency_restore.rs`, replace `.unwrap()` on `journal_path.as_deref()` with an explicit `let Some(path) = input.journal_path.as_deref() else { return Ok(default_clean_outcome) }`. Replace `.unwrap()` on `write_controller_journal_clean` with `?` propagation. Function signature must return `anyhow::Result<AutotuneRestoreOutcome>` (it already does, so `?` is valid throughout).

In `system_context.rs`, replace all `.unwrap()` on parse results with `.unwrap_or_default()` for metrics that have safe zero/empty fallbacks, and `?` with `.context("reading /sys/...")` for reads that must succeed.

In `active_config.rs`, replace `.unwrap()` on `serde_json::to_value` with `.expect("WindowSnapshot is always serializable")` where the invariant is documented, and with `anyhow::bail!` for the two production snapshot paths.

In `startup_recovery.rs`, replace all I/O `.unwrap()` with `?` and surface startup recovery failures as logged warnings that degrade gracefully rather than panicking.

AFFECTED SCOPE:
- `stutter/src/autotune/emergency_restore.rs`
- `stutter/src/autotune/startup_recovery.rs`
- `stutter/src/autotune/system_context.rs`
- `stutter/src/autotune/active_config.rs`

DEPENDENCIES: None. Self-contained.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/autotune/emergency_restore.rs`, find every `.unwrap()` call that is not inside a `#[cfg(test)]` block. For each: if it is on an `Option` that can legitimately be `None` at runtime, replace with a guarded early return or `.unwrap_or`. If it is on an `anyhow::Result` or `std::io::Result`, replace with `?` and add a `.context("description")` qualifier. The function `restore_known_autotune_actions` must not panic on any I/O failure. Apply the same transformation to `stutter/src/autotune/startup_recovery.rs`. In `stutter/src/autotune/system_context.rs`, replace all `.unwrap()` on sysfs parse results with `.unwrap_or_default()` or `.unwrap_or(0)` as appropriate for the field type. Document each with a brief comment explaining the fallback semantics. Do not change test code.

---

PROPOSAL 2: Remove module-level `#![allow(dead_code)]` from `actions/mod.rs`, `community_rules.rs`, and `foreground.rs` by wiring or explicitly removing unused symbols
STATUS: COMPLETED 2026-05-16 — removed module-level suppressors, added targeted TODO-backed allows for intentional forward APIs/test helpers, and kept `cargo check` warning-clean.
PRIORITY: HIGH
Justifies: these suppressors hide divergence between built API surface and actual usage, making it impossible to detect future dead code accumulation via compiler feedback.

CURRENT STATE:
`stutter/src/actions/mod.rs` has `#![allow(dead_code)]` at line 1. The suppressor hides that `RollbackRegistry`, `RollbackHandler`, `RollbackCandidate`, `discover_all()`, `preview_all()`, and `restore_all()` on the registry are never called from autotune or daemon code. The autotune rollback path goes through `emergency_restore.rs` which uses raw `libc` syscalls and action-specific match arms — not the `RollbackRegistry` abstraction.

`stutter/src/community_rules.rs` has `#![allow(dead_code)]` at line 1. `import.rs::RuleImportResult`, `importer.rs::ImportedRule`, `importer.rs::RuleImportContext`, `paths.rs::community_rules_dir` are all unused outside of tests.

`stutter/src/foreground.rs` has `#![allow(dead_code)]` at line 1. `ForegroundWindowSnapshot`, `ForegroundSource`, `ForegroundProviderStatus`, and `ForegroundEvent` are actively used in `focus/mod.rs`, `recorder/mod.rs`, and `session/ui.rs`. The suppressor is a stale artifact.

PROPOSED CHANGE:
**`foreground.rs`**: remove the `#![allow(dead_code)]` suppressor entirely. The module is genuinely used. Run `cargo check` to verify no warnings emerge.

**`community_rules.rs`**: remove the module-level suppressor. Add targeted `#[allow(dead_code)]` only to the specific items in `import.rs` and `importer.rs` that are intentionally forward-declared but not yet wired (`RuleImportResult`, `ImportedRule`, `RuleImportContext`). Each targeted suppressor must carry a `// TODO: wire into community_rules import pipeline` comment.

**`actions/mod.rs`**: remove the module-level suppressor. Evaluate `RollbackRegistry` and its trait: if `RollbackRegistry` is intended to eventually replace the match-arm dispatch in `emergency_restore.rs`, add a `// TODO: replace emergency_restore direct syscall dispatch with RollbackRegistry` comment and add targeted `#[allow(dead_code)]` to the registry API items. If it is not intended to be used, delete it. Do not suppress the whole module.

AFFECTED SCOPE:
- `stutter/src/actions/mod.rs`
- `stutter/src/community_rules.rs`
- `stutter/src/community_rules/import.rs`
- `stutter/src/community_rules/importer.rs`
- `stutter/src/foreground.rs`

DEPENDENCIES: None. Self-contained.

EDIT REQUEST FOR PATCH WRITER:
Remove `#![allow(dead_code)]` from the file headers of `stutter/src/actions/mod.rs`, `stutter/src/community_rules.rs`, and `stutter/src/foreground.rs`. Run `cargo check` (mentally). For any item that becomes a dead_code warning after removal: in `foreground.rs`, there should be none — if there are, investigate. In `community_rules.rs`, add `#[allow(dead_code)] // TODO: wire into import pipeline` to the specific structs in `import.rs` and `importer.rs`. In `actions/mod.rs`, add `#[allow(dead_code)] // TODO: replace emergency_restore direct dispatch` to `RollbackRegistry`, `RollbackHandler`, `RollbackCandidate`, and their methods `discover_all`, `preview_all`, `restore_all`. Do not suppress any other items.

---

PROPOSAL 3: Wire `tasks.rs` dead abstractions or delete them; remove the 9 field-level suppressors
STATUS: COMPLETED 2026-05-16 — chose Option B, deleted the unused refresh-plan abstraction set, and documented the direct `TaskTracker` architecture with an `ARCH` comment.
PRIORITY: HIGH
Justifies: `TargetRefreshPlan`, `TargetMapApplier`, and `TreeEventBuilder` are the designed API for reactive task-tree updates but are completely bypassed in the daemon tick, creating invisible architectural debt.

CURRENT STATE:
`stutter/src/tasks.rs` has `#[allow(dead_code)]` on: `TargetRefreshPlan` (line 26), `TargetRefreshValidation` (line 35), `TaskReplacement` (line 43), `TargetMapOperation` (line 51), `TargetRefreshOutcome` (line 58), `TargetMapApplier` (line 65), `TargetMapApplier::apply` (line 69), `TreeEventBuilder` (line 84), `TreeEventBuilder::events_for_plan` (line 88). The daemon tick calls `TaskTracker::handle_replacements` directly, bypassing `TargetMapApplier`. `TreeEventBuilder::events_for_plan` produces `TreeEvent` values that carry diff information the session event bus could consume, but does not.

PROPOSED CHANGE:
Decide: the `TargetMapApplier`/`TreeEventBuilder` abstraction either represents a planned migration away from the current direct `handle_replacements` call, or it is dead speculative design.

**Option A (wire):** In `autotune/runtime.rs`, replace the direct call to `task_tracker.handle_replacements(...)` with `TargetMapApplier::apply(plan, ...)`. Implement the missing body of `TargetMapApplier::apply` to call `handle_replacements` internally. Have it return `TargetRefreshOutcome`. Remove all `#[allow(dead_code)]` suppressors from `tasks.rs`.

**Option B (delete):** Remove `TargetRefreshPlan`, `TargetRefreshValidation`, `TaskReplacement`, `TargetMapOperation`, `TargetRefreshOutcome`, `TargetMapApplier`, and `TreeEventBuilder` from `tasks.rs`. Remove all their `#[allow(dead_code)]` suppressors. Keep `TaskTracker` and its methods untouched.

The patch writer must choose Option A if the `TreeEventBuilder::events_for_plan` diff events are needed for a future session event bus integration. Choose Option B if they are not.

AFFECTED SCOPE:
- `stutter/src/tasks.rs`
- `stutter/src/autotune/runtime.rs` (if Option A)

DEPENDENCIES: None. Self-contained.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/tasks.rs`, remove all 9 `#[allow(dead_code)]` attributes from `TargetRefreshPlan`, `TargetRefreshValidation`, `TaskReplacement`, `TargetMapOperation`, `TargetRefreshOutcome`, `TargetMapApplier`, `TargetMapApplier::apply`, `TreeEventBuilder`, and `TreeEventBuilder::events_for_plan`. Then choose: either implement `TargetMapApplier::apply` (currently body-less stub) as a thin wrapper over `TaskTracker::handle_replacements` and update `stutter/src/autotune/runtime.rs` to use it, or delete all those types and their `impl` blocks entirely. Document the choice in a `// ARCH:` comment at the top of `tasks.rs`.

---

PROPOSAL 4: Replace hardcoded `ScoreComparisonConfig` constants with variance-weighted thresholds
STATUS: COMPLETED 2026-05-16 — added sample-count threshold tiers, preserved existing call-site behavior with `None`, and replaced objective veto zero-regression values with computed regression magnitude.
PRIORITY: HIGH
Justifies: the 12.5% minimum improvement threshold is invariant across baseline variance, sample count, window duration, and workload type; it misclassifies real improvements as `Inconclusive` when baselines are noisy, and it misclassifies lucky sample-timing as `Improved` when baselines are stable.

CURRENT STATE:
`stutter/src/autotune/comparison.rs` declares:
```rust
pub(crate) const DEFAULT_SCORE_COMPARISON_CONFIG: ScoreComparisonConfig = ScoreComparisonConfig {
    min_improvement_percent: 12.5,
    max_regression_percent: 7.5,
    max_frame_p99_regression_ms: 2.0,
    max_over_5ms_regression: 0,
};
```
`compare_scores_with_config` receives `data_quality: ExperimentDataQuality` (High/Medium/Low). Low quality → immediate `Regressed`. High/Medium → same fixed threshold applied regardless of how many samples were collected, how long the window ran, or how variable the baseline was. `WindowScore` carries `interval_count` and `scored_samples` which could inform threshold tightening or loosening, but are not used by the comparison function.

`autotune/quality.rs` has `DEFAULT_MIN_SCORED_INTERVALS: usize = 5` and `DEFAULT_MIN_SCORED_SAMPLES: u64 = 100`. These guard entry into High/Medium quality. A window with exactly 100 samples gets the same threshold as one with 10,000 samples.

PROPOSED CHANGE:
Add a `ThresholdPolicy` struct to `autotune/comparison.rs` with three threshold tiers keyed on sample count:

```rust
pub struct ThresholdTier {
    pub min_scored_samples: u64,
    pub min_improvement_percent: f64,
    pub max_regression_percent: f64,
}

pub struct ThresholdPolicy {
    pub tiers: Vec<ThresholdTier>,  // sorted ascending by min_scored_samples
}
```

Default tiers (empirically conservative starting points that can be tuned without changing the struct):
- `< 200 samples`: min_improvement=15.0%, max_regression=5.0% (tighter, fewer samples)
- `200–999 samples`: min_improvement=12.5%, max_regression=7.5% (current behaviour preserved)
- `≥ 1000 samples`: min_improvement=10.0%, max_regression=8.5% (more evidence → accept smaller signals)

Modify `compare_scores_with_config` to accept an optional `ThresholdPolicy`. When provided, select the `ScoreComparisonConfig` based on `input.baseline.scored_samples`. When absent, use the existing default. This is additive — all existing callers continue to work with `None`.

In `autotune/objective.rs`, fix all `ExperimentResult::Regressed { regression_percent: 0.0 }` returns in the objective-level veto functions to instead carry the actual regression magnitude computed from the window scores:

```rust
// Replace:
return Some(ExperimentResult::Regressed { regression_percent: 0.0 });
// With (where baseline and candidate are in scope):
return Some(ExperimentResult::Regressed {
    regression_percent: regression_percent(baseline.score.total, candidate.score.total),
});
```

`regression_percent` is already a free function in `comparison.rs`; make it `pub(crate)` or move it to a shared location accessible from `objective.rs`.

AFFECTED SCOPE:
- `stutter/src/autotune/comparison.rs` (add `ThresholdTier`, `ThresholdPolicy`; modify `compare_scores_with_config` signature)
- `stutter/src/autotune/objective.rs` (fix zero regression_percent in all veto returns)
- `stutter/src/autotune/live_experiment.rs` (pass `ThresholdPolicy` through `compare_keep_result` if it calls `compare_for_objective`)
- Callers in `autotune/runtime.rs` and `autotune/comparison.rs` tests must be updated to pass `None` for backward compat

DEPENDENCIES: None. Self-contained.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/autotune/comparison.rs`, add two new public structs: `ThresholdTier { pub min_scored_samples: u64, pub min_improvement_percent: f64, pub max_regression_percent: f64 }` and `ThresholdPolicy { pub tiers: Vec<ThresholdTier> }`. Add `impl ThresholdPolicy { pub fn default_tiers() -> Self { ... } pub fn config_for_samples(&self, scored_samples: u64) -> ScoreComparisonConfig { ... } }`. The default tiers must be: below 200 samples → min=15.0 max=5.0, 200-999 → min=12.5 max=7.5, 1000+ → min=10.0 max=8.5. Modify `compare_scores_with_config` to accept an additional parameter `threshold_policy: Option<&ThresholdPolicy>`; when `Some`, call `policy.config_for_samples(input.baseline.scored_samples)` to select the config and ignore the passed `config` parameter. All existing call sites must pass `None` so the existing behaviour is preserved by default. Then, in `stutter/src/autotune/objective.rs`, make the `regression_percent` free function from `comparison.rs` visible (add `pub(crate)` to it), and replace every `ExperimentResult::Regressed { regression_percent: 0.0 }` in `objective.rs` with `ExperimentResult::Regressed { regression_percent: crate::autotune::comparison::regression_percent(baseline.score.total, candidate.score.total) }`, using whatever baseline/candidate are in scope at that point in the call stack.

---

PROPOSAL 5: Add `AutotuneObservationBuilder` unit tests and planner-integration golden cases
STATUS: COMPLETED 2026-05-16 — added the four observation-builder bridge tests and three planner golden fixtures covering IRQ, GPU, and memory-pressure `ObjectiveSignals`.
PRIORITY: HIGH
Justifies: `AutotuneObservationBuilder` is the only untested bridge between raw rolling window data and planner decisions; a bug here silently passes wrong observations to every provider.

CURRENT STATE:
`stutter/src/autotune/observation_builder.rs` (436 lines) transforms `AutotuneObservationBuilderInput` (which contains a reference to `RollingWindow` state, task info, system health, etc.) into `AutotuneObservation`. It is called from `autotune/runtime.rs` on every tick. It has zero tests. There are no golden cases that verify "given this rolling window state and task tree, the observation has these properties."

`stutter/src/autotune/planner.rs` has 20 golden cases, but all of them use `build_fixture_observation` which constructs observations manually. The path from `RollingWindow::objective_signals()` → `ObservationBuilder` → `AutotuneObservation` → planner is not tested end-to-end.

PROPOSED CHANGE:
Add a `#[cfg(test)] mod tests` block to `autotune/observation_builder.rs` with at minimum:

1. **`observation_from_game_window_with_high_cpu_latency`**: construct a `RollingWindow` with synthetic `IntervalRecord` values representing >5ms runnable latency on game thread TIDs, verify that the resulting `AutotuneObservation` has `focus_confidence >= DEFAULT_MIN_FOCUS_CONFIDENCE` and `situation == SituationKind::GameCpuSchedulerPressure`.

2. **`observation_preserves_irq_signals`**: push synthetic `IrqEventRecord` entries with nonzero `duration_ns` into a `RollingWindow`, call `objective_signals()`, verify `irq_overlap_count` and `irq_worst_overlap_ns` are populated in the resulting observation.

3. **`observation_builder_focus_falls_back_to_unknown_when_confidence_below_threshold`**: construct an observation with a task tree containing only `TaskClass::Service` processes, verify `focus_is_idle_or_unknown()` returns true.

4. **`observation_builder_protected_tasks_exclude_compositor`**: verify that a `TaskClass::Compositor` process appears in `protected_tasks` of the resulting observation.

Add three planner integration golden cases in `testdata/autotune/planner/` that exercise the `observation_signals` path:
- `game_irq_pressure_signals_present.json`: has `irq_overlap_count` and `irq_worst_overlap_ns` set; expects `irq_affinity` candidate eligible
- `game_gpu_power_limited.json`: has `gpu_power_limited=true` and `gpu_busy_percent` high; expects `gpu_power` candidate
- `browser_memory_pressure.json`: has `memory_pressure_some_avg10_percent` nonzero; expects a relevant candidate or explicit denial

AFFECTED SCOPE:
- `stutter/src/autotune/observation_builder.rs` (add tests)
- `testdata/autotune/planner/` (add 3 JSON fixtures)
- `stutter/src/autotune/planner.rs` `expected_names` list must be updated to include the 3 new fixtures

DEPENDENCIES: None. Self-contained.

EDIT REQUEST FOR PATCH WRITER:
Add a `#[cfg(test)] mod tests` block at the bottom of `stutter/src/autotune/observation_builder.rs`. Add 4 unit tests as described above. Each test must construct a `RollingWindow` using `RollingWindow::new(Duration::from_secs(30))`, push synthetic events via the `push_*` methods, call `RollingWindow::objective_signals()` or `RollingWindow::score()` as appropriate, and assert specific fields on the resulting `ObjectiveSignals` or `AutotuneObservation`. Use the existing `test_fixture_builder.rs` helpers where available; do not introduce new test dependencies. Then add 3 new JSON files to `testdata/autotune/planner/` following the exact schema of existing fixtures (`game_cpu_scheduler_pressure.json` is the reference). Update the `expected_names` vec in the `planner_golden_cases` test in `stutter/src/autotune/planner.rs` to include the 3 new fixture names in alphabetical order.

---

PROPOSAL 6: Add a `ControllerStateMachine` integration test covering the full Observing→Apply→Keep/Revert→Cooldown cycle
STATUS: COMPLETED 2026-05-16 — added `stutter/tests/autotune_lifecycle.rs`, runtime state accessors, temp journal wiring, and fake simulated rollback cleanup through emergency restore.
PRIORITY: HIGH
Justifies: the controller state machine's 20+ inline unit tests each test one transition in isolation; no test exercises the full cycle with a real `AutotuneRuntime` tick sequence.

CURRENT STATE:
`autotune/controller.rs` has 20+ `#[test]` blocks each calling `decide_autotune_transition` with a manually-constructed `ControllerRuntimeState`. These are correct and well-written. However, the higher-level `AutotuneRuntime::on_tick` path — which calls `decide_autotune_transition` using state derived from a real `RollingWindow`, `CandidatePlanner`, and `apply_candidate_with_audit` — has no comparable test. `autotune/runtime.rs` has several inline tests (at lines 2076–2808) but they use `simulate_action_effects: true` which bypasses the actual apply/rollback logic. No test drives the runtime through a full experiment lifecycle.

`daemon/soak.rs:run_fake_daemon_soak` is a pure simulation with hardcoded per-tick increments, not a real runtime.

PROPOSED CHANGE:
Add a standalone integration test file `stutter/tests/autotune_lifecycle.rs`. This file must:

1. Construct an `AutotuneRuntimeConfig` with `simulate_action_effects: true`, `daemon_config.mode = DaemonMode::ApplyLowRisk`, a `Fake` candidate with `SafetyClass::ReversibleLowRisk`, and a temporary directory for journal/history paths.

2. Construct an `AutotuneRuntime` and feed it synthetic `MonitorEvent` ticks via the mpsc channel.

3. Assert the following state sequence:
   - After baseline window fills: `controller_state.phase == ControllerPhase::Observing`
   - After planner selects the fake candidate: `phase == ControllerPhase::Applying` or equivalent
   - After candidate measurement window: experiment result is `Improved` (since `simulate_action_effects: true` freezes windows)
   - After keep decision: `kept_candidate.current.is_some()`
   - After a forced revert trigger: rollback token is consumed and phase returns to `Observing`

4. Assert that the journal file contains a clean record after the full cycle.

AFFECTED SCOPE:
- New file: `stutter/tests/autotune_lifecycle.rs`

DEPENDENCIES: Proposal 1 (the test must not panic on I/O errors in startup_recovery, which runs on construction).

EDIT REQUEST FOR PATCH WRITER:
Create the file `stutter/tests/autotune_lifecycle.rs`. It must import from `stutter::{autotune::runtime::{AutotuneRuntime, AutotuneRuntimeConfig}, daemon::{DaemonConfig, DaemonMode}, actions::{ActionSource, SafetyClass, ActionId}, autotune::candidate::CandidateAction, session_events::MonitorEvent}`. Construct an `AutotuneRuntimeConfig` with `simulate_action_effects: true`, `daemon_config.mode = DaemonMode::ApplyLowRisk`, `simulated_candidates` containing one `CandidateAction::Fake { action_id: ActionId("test-fake".to_owned()), safety_class: SafetyClass::ReversibleLowRisk }`, and temp paths for journal/history. Drive the runtime through at least 60 synthetic tick events by calling `runtime.on_tick(MonitorEvent::Tick { ... })` in a loop (use `tokio::test`). After the loop, assert: `runtime.controller_state().phase` is not `ControllerPhase::Faulted`, the history log exists and contains at least one entry, and the journal is in a clean state. The test must complete without panicking.

---

PROPOSAL 7: Add targeted `#[allow(dead_code)]` with wiring TODOs to `diagnosis.rs` dead evidence fields
STATUS: COMPLETED 2026-05-16 — removed the stale `diagnosis.rs` dead-code attributes and added the advisor TODO note; the cited evidence fields are not present in the current code shape.
PRIORITY: MEDIUM
Justifies: suppressed fields on `DiagnosisCandidate` and `LiveDiagnosisEntry` hide that the advisor does not consume all diagnosis evidence, which creates silent information loss in the recommendation pipeline.

CURRENT STATE:
`stutter/src/diagnosis.rs` lines 68, 78, 253, 263 suppress dead fields. Specifically, `DiagnosisCandidate.evidence_details: Vec<String>` (line 68 area) and `LiveDiagnosisEntry.raw_latencies: Vec<u64>` (line 263 area) are never read by `advisor.rs` or `recommend.rs`. These carry per-event evidence strings and latency samples that the advisor would need to produce specific recommendations (e.g., "IRQ 44 on CPU 2 caused 3ms stutter 7 times").

PROPOSED CHANGE:
Remove the four `#[allow(dead_code)]` attributes from `diagnosis.rs`. For fields that become warnings, add targeted `#[allow(dead_code)] // TODO: consumed by advisor when evidence-detail recommendations are implemented` inline. Add a comment in `advisor.rs` at the top: `// TODO: consume DiagnosisCandidate::evidence_details for specific actionable recommendations`.

AFFECTED SCOPE:
- `stutter/src/diagnosis.rs`
- `stutter/src/advisor.rs` (comment only)

DEPENDENCIES: None.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/diagnosis.rs`, remove the 4 `#[allow(dead_code)]` attributes at lines 68, 78, 253, and 263. For each field that emits a dead_code warning after removal, add `#[allow(dead_code)] // TODO: consumed by advisor in evidence-detail recommendation pass` immediately before that field. Do not remove or change any field values. Add a block comment at the top of `stutter/src/advisor.rs` reading `// TODO: DiagnosisCandidate::evidence_details and LiveDiagnosisEntry::raw_latencies are not yet consumed here. When implementing specific actionable recommendations, read these fields to produce per-IRQ/per-process evidence strings.`

---

PROPOSAL 8: Rename `rolling_window::WindowScore` to `RollingWindowScore` to eliminate namespace collision with `experiment::WindowScore`
STATUS: COMPLETED 2026-05-16 — renamed `rolling_window::WindowScore` to `RollingWindowScore` and updated the observation-builder import; `experiment::WindowScore` remains unchanged.
PRIORITY: MEDIUM
Justifies: two public structs named `WindowScore` in sibling modules of the same crate will cause import confusion as the codebase grows; one is already causing implicit type-hiding in `rolling_window.rs` line 20.

CURRENT STATE:
`stutter/src/autotune/rolling_window.rs` line 20 declares `pub struct WindowScore` with fields: `duration_ms: u64`, `interval_count: usize`, `scored_task_count: usize`, `scored_samples: u64`, `score_total: u64`, `over_1ms: u64`, `over_2ms: u64`, `over_5ms: u64`, `max_latency_ns: u64`, `frame_count: usize`, `frame_p99_ms: f64`, `frame_max_ms: f64`, `data_quality: OnlineDataQuality`.

`stutter/src/autotune/experiment.rs` line 23 declares `pub struct WindowScore` with fields: `started_unix_nanos: u128`, `finished_unix_nanos: u128`, `interval_count: usize`, `scored_samples: u64`, `scored_task_count: usize`, `score: StutterScore`.

`comparison.rs` imports `experiment::WindowScore`. `rolling_window.rs` defines its own. Callers who `use crate::autotune::rolling_window::*` and `use crate::autotune::experiment::*` will have a silent collision resolved by whichever import is last.

PROPOSED CHANGE:
Rename `rolling_window::WindowScore` to `rolling_window::RollingWindowScore`. Update all references within `rolling_window.rs` and any callers that import specifically from `rolling_window`. The `experiment::WindowScore` retains its name as it is the canonical type used by `comparison.rs`, `objective.rs`, `live_experiment.rs`, and `controller.rs`.

AFFECTED SCOPE:
- `stutter/src/autotune/rolling_window.rs` (rename struct, all references)
- Any file that imports `WindowScore` from `rolling_window` — check by grepping `rolling_window::WindowScore` and `use.*rolling_window.*WindowScore`

DEPENDENCIES: None.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/autotune/rolling_window.rs`, rename the struct `WindowScore` to `RollingWindowScore`. Update every use of `WindowScore` within that file, including the `#[allow(dead_code)]` annotation at line 20 (which should be retargeted to `RollingWindowScore`), the return type of `score_with_quality_policy`, and all internal references. Then grep for any file in `stutter/src/` that imports `WindowScore` from `rolling_window` or uses the fully-qualified path `rolling_window::WindowScore`, and update those to `rolling_window::RollingWindowScore`. Do not rename `experiment::WindowScore`.

---

PROPOSAL 9: Wire `StutterError` at actual error-origination callsites or delete it
STATUS: COMPLETED 2026-05-16 — chose Option A because `ConfigError` is actively used; wired `StutterError` through `main`, `run_cli`, and command dispatch with a typed command-boundary fallback.
PRIORITY: MEDIUM
Justifies: the typed error enum in `error.rs` exists solely for documentation value right now; it adds maintenance overhead for zero runtime benefit.

CURRENT STATE:
`stutter/src/error.rs` defines `StutterError` with `#[from]` impls for `ConfigError`, `TargetError`, `EbpfError`, `ProbeError`, `RecordingError`, `ArtifactError`, `ReportError`, `RemoteError`. None of these are used at callsites. The entire production codebase returns `anyhow::Result<_>`. `StutterError` is not the return type of `main()` or any public API boundary.

PROPOSED CHANGE:
**Option A (wire, correct approach):** Change `fn main() -> anyhow::Result<()>` to `fn main() -> Result<(), StutterError>`. Change the top-level command dispatch functions in `commands/` to return `Result<(), StutterError>`. This gives the `#[from]` impls a purpose and ensures errors are typed at the API boundary. Internal functions continue using `anyhow::Result`.

**Option B (delete):** Delete `error.rs` entirely. Remove `pub mod error` from `lib.rs`. This is the correct choice if typed errors at the CLI boundary are not valuable for this tool.

AFFECTED SCOPE:
- `stutter/src/error.rs`
- `stutter/src/main.rs` (if Option A)
- `stutter/src/commands/mod.rs` and submodules (if Option A)
- `stutter/src/lib.rs`

DEPENDENCIES: None.

EDIT REQUEST FOR PATCH WRITER:
Choose Option B unless there is a concrete downstream consumer (Prometheus exporter, machine-readable error output, library consumer) that requires typed errors. If choosing Option B: delete `stutter/src/error.rs`, remove `pub mod error;` from `stutter/src/lib.rs`, and remove any `use crate::error::*` imports. Confirm no file other than `error.rs` itself references `StutterError`, `ConfigError`, `TargetError`, `EbpfError`, `ProbeError`, `RecordingError`, `ArtifactError`, `ReportError`, or `RemoteError` in production code (only in test code or the file itself). If other files do reference these, document them before deleting.

---

PROPOSAL 10: Add `AutotuneObservationBuilder` → planner end-to-end scenario tests for the 3 signal paths not covered by golden cases
STATUS: COMPLETED 2026-05-16 — extended planner golden fixtures with optional `hardware_signals` deserialization and used it for the three new signal-path fixtures.
PRIORITY: MEDIUM
Justifies: IRQ affinity, GPU power, and memory pressure candidates are selected based on `ObjectiveSignals` fields that come from `rolling_window::objective_signals()`, a path not exercised by any current test.

CURRENT STATE:
The 20 planner golden cases in `testdata/autotune/planner/` construct `AutotuneObservation` via `build_fixture_observation`, which hardcodes `gpu_power_evidence: bool` and similar fields directly — bypassing `rolling_window::objective_signals()` entirely. The providers `irq_affinity.rs`, `gpu_power.rs`, and `vm_knob.rs` read `ObjectiveSignals` from the observation's `hardware_signals` field. If `rolling_window::objective_signals()` returns wrong values (wrong thresholds, missing fields), the planner will silently not generate candidates without any test failing.

PROPOSED CHANGE:
See Proposal 5. This proposal specifically calls out that the 3 new golden cases in Proposal 5 must be constructed by calling `rolling_window::objective_signals()` in their fixture builder, not by hardcoding signal values in the JSON. The fixture builder in `planner.rs::build_fixture_observation` must be extended to accept an `objective_signals: Option<ObjectiveSignals>` field in `PlannerGoldenCase`, and use it if present.

AFFECTED SCOPE:
- `stutter/src/autotune/planner.rs` (extend `PlannerGoldenCase` struct and `build_fixture_observation`)
- `testdata/autotune/planner/` (3 new fixtures from Proposal 5)

DEPENDENCIES: Proposal 5 must be completed first; this extends the infrastructure created there.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/autotune/planner.rs`, extend the `PlannerGoldenCase` struct (in the `#[cfg(test)]` section) with an optional field `hardware_signals: Option<serde_json::Value>`. In `build_fixture_observation`, if `hardware_signals` is `Some`, deserialize it as `ObjectiveSignals` and assign it to `observation.hardware_signals`. Update the `PlannerGoldenCase` JSON schema comment. Then, for the 3 new fixture files added in Proposal 5, include a `hardware_signals` JSON object that directly populates the relevant `ObjectiveSignals` fields (e.g., `irq_overlap_count`, `gpu_busy_percent`, `memory_pressure_some_avg10_percent`) rather than relying on the boolean flag shortcuts.

---

PROPOSAL 10.5: Implement active CPU-affinity profile matching
STATUS: COMPLETED 2026-05-16 — implemented CPU-affinity profile active-state matching against active task snapshots and `ActiveConfigSnapshot.affinity.per_tid`, and added planner no-op/drift tests.

PRIORITY: CRITICAL
CPU-affinity profiles are the core currently mature apply family, but live no-op/external-mutation detection still cannot verify them.

CURRENT STATE:
`stutter/src/autotune/active_config.rs` implements `CandidateAction::matches_active_config(&ActiveConfigSnapshot)`. It compares active config for nice, ionice, uclamp, cgroup, IRQ, CPU power, GPU power, and VM knobs, but for `CandidateAction::CpuAffinityProfile` it returns `ActiveConfigMatch::Unknown` with the exact summary text: `"active per-profile CPU affinity matching is not implemented"`. 
`CandidateAction::CpuAffinityProfile` now stores a `CpuAffinityProfilePlan` with `profile_name`, `profile`, and `tree_pid`; `CpuAffinityProfilePlan` declares action kind `cpu_affinity_profile`, effect scope `LocalProcessTree`, objective `StutterScore`, and conflict group `CpuPlacement`. 
Planner no-op detection depends on `candidate.matches_active_config(snapshot)`: if it returns `Matches`, the planner adds `CandidateDenyReason::NoEffectiveChange`. Because CPU-affinity profiles return `Unknown`, the planner cannot deny no-op CPU-affinity candidates or detect kept CPU-affinity drift. 

PROPOSED CHANGE:
Implement CPU-affinity profile matching in `active_config.rs` by evaluating the planned profile rules against `ActiveConfigSnapshot.affinity.per_tid` and `AutotuneObservation.active_tasks`. Add a reusable function:

```rust
pub fn cpu_affinity_profile_match(
    plan: &CpuAffinityProfilePlan,
    snapshot: &ActiveConfigSnapshot,
    active_tasks: &[ActiveTaskSnapshot],
) -> ActiveConfigMatch
```

Because `matches_active_config(&self, snapshot)` currently has no access to active task snapshots, replace it with either:

```rust
pub struct ActiveConfigMatchInput<'a> {
    pub snapshot: &'a ActiveConfigSnapshot,
    pub active_tasks: &'a [ActiveTaskSnapshot],
}

pub fn matches_active_config(&self, input: ActiveConfigMatchInput<'_>) -> ActiveConfigMatch
```

or add a CPU-affinity-specific path in planner where the observation is available. The matcher must:

* Compile/apply profile rule matching using the same semantics as `CpuAffinityProfileAction::dry_run()` / `apply()`.
* For each mutable target task matched by the profile, compare the requested mask to `snapshot.affinity.per_tid[tid]`.
* Return `Matches` only if every planned affected task already has the requested mask.
* Return `Differs` if any planned affected task has a different active mask.
* Return `Unknown` if no target task information is available or required active affinity data is missing.
* Preserve profile rule order and first-match-wins behavior.
* Add tests for exact match, one differing TID, missing affinity data, no matched tasks, excluded/protected tasks, and broad fallback rules.

AFFECTED SCOPE:

* `stutter/src/autotune/active_config.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/candidate.rs`
* `stutter/src/actions/cpu_affinity.rs`
* `stutter/src/profiles.rs`
* planner tests and active-config tests
  This is a medium ripple because the matching API must receive active task snapshots.

DEPENDENCIES:

* Must be done before PROPOSAL 11, PROPOSAL 12, and any broader autonomous apply expansion.

EDIT REQUEST FOR PATCH WRITER:
Implement CPU-affinity profile active-state matching. Currently `CandidateAction::CpuAffinityProfile` returns `ActiveConfigMatch::Unknown` in `stutter/src/autotune/active_config.rs`. Replace that placeholder with rule-accurate matching against active task snapshots and `ActiveConfigSnapshot.affinity.per_tid`. Update the planner call sites so CPU-affinity matching has access to active task snapshots. Add tests proving no-op CPU-affinity profiles are denied and externally changed kept CPU-affinity profiles are detected.

---

PROPOSAL 11: Unify CPU-affinity rule evaluation between profile apply, dry-run, candidate generation, and active matching
STATUS: COMPLETED 2026-05-16 — added a shared first-match profile evaluator with per-task requested mask metadata and reused it for active-config matching while preserving existing apply/dry-run semantics.

PRIORITY: HIGH
CPU-affinity logic must not be duplicated across apply, dry-run, generation, and active-state comparison because small semantic drift will make rollback/no-op detection incorrect.

CURRENT STATE:
`CandidateAction::CpuAffinityProfile` wraps a full `Profile` inside `CpuAffinityProfilePlan`. 
`ActiveConfigSnapshot` currently cannot match that profile to active affinity state. 
Planner uses `NoEffectiveChange`, `ExternalMutationDetected`, and `KeptActionNoLongerActive` based on `matches_active_config()`, so any divergence between apply semantics and active matching semantics will directly corrupt planner decisions. 

PROPOSED CHANGE:
Create a shared module:

```rust
stutter/src/profiles/evaluate.rs
```

or:

```rust
stutter/src/actions/cpu_affinity/profile_eval.rs
```

with:

```rust
pub struct ProfileEvaluationInput<'a> {
    pub profile: &'a Profile,
    pub active_tasks: &'a [ActiveTaskSnapshot],
    pub topology: Option<&'a TopologyModel>,
}

pub struct ProfileTaskPlan {
    pub tid: u32,
    pub process_pid: u32,
    pub comm: String,
    pub class: TaskClass,
    pub requested_mask: String,
    pub matched_rule_index: usize,
    pub matched_rule_name: Option<String>,
}

pub fn evaluate_profile_for_tasks(input: ProfileEvaluationInput<'_>) -> Vec<ProfileTaskPlan>
```

Migrate CPU-affinity apply/dry-run and new active matching to call this shared evaluator. Remove duplicated rule matching where it exists. The evaluator must preserve existing profile semantics:

* file-order first matching rule wins
* `match_comm` literal/regex behavior
* `match_class`
* explicit masks
* any existing priority/nice/ionice side effects must remain separate from CPU mask evaluation

AFFECTED SCOPE:

* `stutter/src/actions/cpu_affinity.rs`
* `stutter/src/profiles.rs`
* possible new `stutter/src/profiles/evaluate.rs`
* `stutter/src/autotune/active_config.rs`
* `stutter/src/autotune/candidate.rs`
* `stutter/src/autotune/providers/cpu_affinity.rs`
* CPU-affinity tests/profile tests
  This is a medium-to-large refactor but is required to make CPU-affinity autonomous behavior trustworthy.

DEPENDENCIES:

* Should be implemented immediately before or together with PROPOSAL 10.
* Blocks safe expansion of apply-low-risk profile retention.

EDIT REQUEST FOR PATCH WRITER:
Extract CPU-affinity profile rule evaluation into one shared evaluator. Replace profile matching logic in apply/dry-run/candidate active matching with this evaluator. The evaluator must produce per-task planned affinity decisions with matched rule metadata. Add regression tests proving dry-run affected tasks and active-config matching use identical task/rule/mask decisions.

---

PROPOSAL 12: Make candidate plan files executable for CPU-affinity profiles or explicitly non-plan-based
STATUS: COMPLETED 2026-05-16 — chose Option B, added manual-only candidate-plan metadata for CPU-affinity profiles, and made generic candidate-plan apply reject them with `candidate_plan_manual_only`.

PRIORITY: HIGH
The project now writes candidate plan files, but CPU-affinity profile candidates have no executable payload in that schema even though they are the primary mature action family.

CURRENT STATE:
`CandidatePlanFile::from_candidate()` writes a plan with `descriptor`, `objective`, `evidence`, and `executable: CandidateExecutablePlan::from_candidate(candidate)`. 
`CandidateExecutablePlan` supports `Nice`, `IoPrio`, `Uclamp`, and `CgroupPlacement`. It returns `None` for `CpuAffinityProfile`, IRQ, CPU power, GPU power, VM knob, and fake candidates. 
CPU-affinity profiles still have separate legacy/manual paths through `apply-profile`, while generic candidate plan files cannot represent the profile payload. This creates two manual-apply paths: profile file/manual command for CPU affinity, JSON executable plan for process-local medium-risk actions.

PROPOSED CHANGE:
Choose one of these two designs and enforce it consistently:

Option A, preferred:
Add CPU-affinity profile executable support:

```rust
CandidateExecutablePlan::CpuAffinityProfile {
    profile: Profile,
    tree_pid: u32,
}
```

Update `CandidateExecutablePlan::from_candidate()` and `into_candidate()` accordingly. Ensure the plan file includes enough data to reconstruct the profile exactly. Add policy validation to reject stale/root PID-invalid plans.

Option B:
Make CPU-affinity candidate plan files explicitly non-executable and add stable fields:

```rust
manual_apply_command: "stutter apply-profile ..."
executable: null
manual_only_reason: "cpu-affinity profiles use apply-profile, not candidate-plan apply"
```

Then generic candidate-plan apply must reject CPU-affinity plan files with a stable reason code rather than silently writing `executable: None`.

AFFECTED SCOPE:

* `stutter/src/autotune/candidate.rs`
* `stutter/src/autotune/human_output.rs`
* `stutter/src/commands/autotune.rs`
* `stutter/src/cli.rs`
* `stutter/src/autotune/apply.rs`
* docs for candidate plan files
  Self-contained if Option B; medium ripple if Option A.

DEPENDENCIES:

* Should follow PROPOSAL 10 and PROPOSAL 11 if Option A needs profile evaluation.
* Should precede any user-facing candidate-plan workflow expansion.

EDIT REQUEST FOR PATCH WRITER:
Resolve the split between CPU-affinity profile suggestions and generic candidate plan files. Currently `CandidateExecutablePlan` excludes CPU-affinity profiles. Either add `CpuAffinityProfile` to `CandidateExecutablePlan` and make plan files executable for CPU-affinity candidates, or make CPU-affinity plan files explicitly manual-only with a stable rejection reason in candidate-plan apply. Add tests for serialization, deserialization, apply rejection/apply reconstruction, and stale PID handling.

---

PROPOSAL 13: Populate missing live objective signals instead of leaving them as `None`
STATUS: COMPLETED 2026-05-16 — populated rolling-window memory/swap/writeback/GPU identity and limit-source signals, added objective signal quality metadata, made missing required objective evidence inconclusive, and lowered provider confidence for missing-quality evidence.

PRIORITY: CRITICAL
Objective verification exists, but several critical live signals remain unpopulated, so providers and keep/revert logic cannot reach full-system-tuner reliability.

CURRENT STATE:
`ObjectiveSignals` defines fields for block I/O, IRQ, thermal, CPU power, GPU power, render node, memory pressure, swap activity, dirty writeback, frame p99, and foreground latency. 
`RollingWindow` owns interval, frame, diagnosis, IRQ, block I/O, GPU, CPU frequency, and foreground event queues. 
`RollingWindow::objective_signals()` currently sets `gpu_active_render_node: None`, `memory_pressure_some_avg10_percent: None`, and `swap_activity_events: None`; it only sets dirty writeback when detected, and GPU render-node identity is not connected to focus/workload routing. 

PROPOSED CHANGE:
Extend live signal collection so these fields are populated:

* `gpu_active_render_node`
* `memory_pressure_some_avg10_percent`
* `swap_activity_events`
* `dirty_writeback_events` with actual count, not only optional presence
* CPU power limit source and affected policy
* GPU power limit reason if available
* block I/O overlap basis/trust level
* IRQ overlap basis/trust level

Add `ObjectiveSignalSourceQuality`:

```rust
pub enum ObjectiveSignalQuality {
    Direct,
    Derived,
    Approximate,
    Missing,
}
```

and include quality in either `ObjectiveSignals` or a parallel `ObjectiveSignalQualitySnapshot`.

Update missing-signal behavior:

* Providers must refuse or lower confidence when required signals are `Missing`.
* `compare_for_objective()` must distinguish “missing required signal” from “signal says no improvement.”
* Status output must show missing objective signals as structured diagnostic reasons.

AFFECTED SCOPE:

* `stutter/src/autotune/objective.rs`
* `stutter/src/autotune/rolling_window.rs`
* `stutter/src/recorder.rs`
* `stutter/src/session_events.rs`
* `stutter/src/hwmon.rs`
* `stutter/src/autotune/providers/gpu_power.rs`
* `stutter/src/autotune/providers/vm_knob.rs`
* `stutter/src/autotune/providers/cpu_power.rs`
* `stutter/src/autotune/providers/irq_affinity.rs`
* report/analysis JSON if objective signals are exported
  Large telemetry/verification ripple.

DEPENDENCIES:

* Should be implemented before trusting high-risk suggestions or medium-risk keep/revert decisions.
* Blocks PROPOSAL 14, PROPOSAL 15, PROPOSAL 16, and PROPOSAL 17.

EDIT REQUEST FOR PATCH WRITER:
Populate all currently missing `ObjectiveSignals` fields from live telemetry. `RollingWindow::objective_signals()` must no longer hardcode `gpu_active_render_node`, memory pressure, and swap activity to `None` when the required source data exists. Add signal quality metadata, propagate it into providers and objective comparison, and add tests proving missing required signals block or lower-confidence candidates.

---

PROPOSAL 14: Implement focused-GPU ownership resolution for multi-GPU systems
STATUS: COMPLETED 2026-05-16 — added `autotune::gpu_focus`, resolved focused render nodes from target FDs/sample identity/single-GPU fallback, propagated focus confidence/source into objective signals, and guarded multi-GPU provider selection on that confidence.

PRIORITY: HIGH
GPU power tuning must never select the wrong GPU on multi-GPU systems.

CURRENT STATE:
`GpuPowerProvider::selected_gpu()` uses `objective_signals.gpu_active_render_node` if present; otherwise it only selects a GPU when there is exactly one DRM device. With multiple DRM devices and no render-node signal, it returns `None`. 
`RollingWindow::objective_signals()` currently sets `gpu_active_render_node: None`. 
Therefore, on multi-GPU systems, GPU power suggestions depend on a signal that is not currently populated.

PROPOSED CHANGE:
Create a module:

```rust
stutter/src/autotune/gpu_focus.rs
```

with:

```rust
pub struct FocusGpuResolver;
pub struct FocusGpuResolution {
    pub render_node: Option<String>,
    pub drm_card: Option<String>,
    pub pci_id: Option<String>,
    pub confidence: f32,
    pub source: FocusGpuSource,
}
```

Resolution sources:

* target process open FDs under `/proc/<pid>/fd` pointing to `/dev/dri/renderD*`
* MangoHud/GPU sample device identity if present
* hwmon/DRM card selected by monitor flags
* explicit config override
* fallback only if single GPU exists

Update `ObjectiveSignals.gpu_active_render_node` from this resolver. Update `GpuPowerProvider` to require `FocusGpuResolution.confidence >= policy threshold` for multi-GPU systems.

AFFECTED SCOPE:

* new `stutter/src/autotune/gpu_focus.rs`
* `stutter/src/autotune/rolling_window.rs`
* `stutter/src/autotune/observation_builder.rs`
* `stutter/src/system_inventory.rs`
* `stutter/src/autotune/providers/gpu_power.rs`
* monitor config/docs for explicit GPU override
  Medium-large hardware routing change.

DEPENDENCIES:

* Requires PROPOSAL 13.
* Must be done before GPU power suggestions are considered trustworthy.

EDIT REQUEST FOR PATCH WRITER:
Add focused-GPU resolution. Populate `ObjectiveSignals.gpu_active_render_node` by inspecting focused target process DRM render-node usage and configured GPU selection. Update `GpuPowerProvider` so multi-GPU systems require a resolved focused GPU. Add tests for one GPU, two GPUs with focused render node, two GPUs without render node, and explicit override.

---

PROPOSAL 15: Add AC/battery and thermal-headroom gates to CPU power provider

STATUS: COMPLETED 2026-05-16 — added `PowerSourceSnapshot` inventory telemetry, `autotune.allow_cpu_power_on_battery`, CPU-power provider battery/thermal gates with evidence/confidence, docs, and focused tests for AC, battery, desktop, thermal, and already-performance cases.

PRIORITY: HIGH
CPU power tuning must not push performance governor/EPP when the machine lacks power or thermal headroom.

CURRENT STATE:
`CpuPowerProvider` builds `CpuPowerCandidateEvidence` with `ac_power: Option<bool>`, but `cpu_power_evidence()` always sets `ac_power: None`. 
The provider only checks `input.system_health.ok_for_apply`, `objective_signals.cpu_power_limited == Some(true)`, available governors, related CPUs, and whether the current governor/EPP is already performance. 
The confidence calculation includes thermal headroom and CPU limit evidence, but cannot incorporate AC power because it is never collected. 

PROPOSED CHANGE:
Add power-source telemetry:

* AC online status from `/sys/class/power_supply/*/online`
* battery discharging/charging status from `/sys/class/power_supply/*/status`
* optional config override for desktop systems without batteries

Add to `SystemInventory` or `SystemContextSnapshot`:

```rust
pub struct PowerSourceSnapshot {
    pub ac_online: Option<bool>,
    pub battery_present: bool,
    pub battery_discharging: Option<bool>,
}
```

Update CPU power provider:

* Do not propose performance governor/EPP when battery is discharging unless explicit config allows it.
* Require thermal headroom.
* Include AC/battery status in evidence and confidence.
* Add policy config: `allow_cpu_power_on_battery = false` default.

AFFECTED SCOPE:

* `stutter/src/system_inventory.rs`
* `stutter/src/autotune/system_context.rs`
* `stutter/src/autotune/providers/cpu_power.rs`
* `stutter/src/daemon/config.rs`
* `stutter/src/config/model.rs`
* `stutter/src/config/schema.rs`
* docs and provider tests
  Medium system-context/provider ripple.

DEPENDENCIES:

* Should follow PROPOSAL 13.
* Required before CPU power suggestions are trusted.

EDIT REQUEST FOR PATCH WRITER:
Collect AC/battery state and feed it into `CpuPowerProvider`. Replace `ac_power: None` with real power-source evidence. Block CPU power performance candidates while on battery by default. Add config override, evidence output, confidence integration, and tests for AC online, battery discharging, no battery desktop, thermal degraded, and already-performance policy.

---

PROPOSAL 16: Expand VM knob provider beyond fixed swappiness and add knob-specific policies

STATUS: COMPLETED 2026-05-16 — replaced the single fixed swappiness path with a VM knob policy table, added direct swap/writeback triggers, rollback/current/proposed evidence, bytes-vs-ratio conflict suppression, expanded VM inventory, and high-risk/manual-only provider tests.

PRIORITY: MEDIUM
The VM provider currently represents the entire VM tuning surface as one fixed `vm.swappiness=10` proposal.

CURRENT STATE:
`VmKnobProvider` only constructs one candidate named `"vm-swappiness-investigate-10"`, writing `proc/sys/vm/swappiness` to `"10"`. 
It requires at least one of memory pressure, swap activity, or dirty writeback evidence, and refuses if current swappiness is already `10`. 
`ObjectiveSignals` has fields for memory pressure, swap activity, and dirty writeback, but some are currently unpopulated by rolling-window live signals.

PROPOSED CHANGE:
Add a VM tuning policy table:

```rust
pub struct VmKnobPolicy {
    pub knob: &'static str,
    pub safe_values: Vec<String>,
    pub trigger: VmKnobTrigger,
    pub objective: ObjectiveKind,
    pub safety_class: SafetyClass,
    pub manual_only: bool,
}
```

Initial supported suggestions:

* `vm.swappiness` for swap-heavy interactive workloads
* `vm.dirty_background_ratio` or `dirty_background_bytes` for dirty writeback stalls
* `vm.dirty_ratio` or `dirty_bytes` for writeback pressure
* no transparent hugepage changes unless separately modeled

Rules:

* Do not suggest mutually exclusive ratio/bytes knobs simultaneously.
* Do not suggest sysctl changes without direct evidence.
* Do not apply VM knobs autonomously.
* Include current value, proposed value, trigger evidence, and rollback value in evidence.

AFFECTED SCOPE:

* `stutter/src/autotune/providers/vm_knob.rs`
* `stutter/src/actions/vm_knobs.rs`
* `stutter/src/autotune/objective.rs`
* `stutter/src/system_inventory.rs`
* docs/config for VM tuning
  Medium provider/action expansion.

DEPENDENCIES:

* Requires PROPOSAL 13 for memory/writeback/swap signals.
* Must remain manual-only until high-risk apply support exists.

EDIT REQUEST FOR PATCH WRITER:
Refactor `VmKnobProvider` from a single hardcoded swappiness proposal into a knob-policy-driven provider. Add knob-specific triggers, evidence, rollback value capture, and mutual-exclusion rules. Keep all VM knob candidates manual-only/high-risk. Add tests for swap pressure, dirty writeback pressure, already-target-value no-op, missing evidence, and conflicting ratio/bytes knobs.

---

PROPOSAL 17: Add IRQ CPU-placement policy that avoids moving IRQs onto protected/focused CPUs blindly

STATUS: COMPLETED 2026-05-16 — added `CpuPlacementMap`, IRQ target selection that prefers housekeeping CPUs, excludes audio/compositor/reserved/focused CPUs by default, respects candidate IRQ CPU masks, and emits placement rationale with provider tests.

PRIORITY: HIGH
IRQ affinity suggestions currently choose the least-busy CPU from IRQ counters, but a full tuner must account for focused workload placement and protected CPU roles.

CURRENT STATE:
`IrqAffinityProvider` selects a hot IRQ from structured objective signals, looks up current IRQ affinity from active config, finds an IRQ line in inventory, then calls `least_busy_cpu(irq_line)` or falls back to `signals.irq_hot_cpu`; it converts that CPU to a single-CPU mask. 
The provider does not consult focused workload CPU placement, CPU topology role, isolated/reserved cores, compositor/audio CPU reservations, or current CPU-affinity profile intent. 

PROPOSED CHANGE:
Add:

```rust
pub struct CpuPlacementMap {
    pub focused_workload_cpus: BTreeSet<u32>,
    pub compositor_cpus: BTreeSet<u32>,
    pub audio_realtime_cpus: BTreeSet<u32>,
    pub housekeeping_cpus: BTreeSet<u32>,
    pub reserved_cpus: BTreeSet<u32>,
    pub candidate_irq_cpus: BTreeSet<u32>,
}
```

Use it in IRQ provider:

* Never suggest moving IRQs to audio realtime CPUs.
* Prefer housekeeping CPUs when available.
* Avoid focused render/game CPUs unless IRQ belongs to the focused device and overlap evidence says current placement is worse.
* Consider SMT siblings if topology is available.
* Do not suggest single-CPU mask if target CPU is outside allowed IRQ candidate set.
* Include placement rationale in evidence.

AFFECTED SCOPE:

* `stutter/src/autotune/providers/irq_affinity.rs`
* `stutter/src/autotune/system_context.rs`
* `stutter/src/topology.rs`
* `stutter/src/autotune/active_config.rs`
* `stutter/src/autotune/observation.rs`
* provider tests
  Medium provider/topology change.

DEPENDENCIES:

* Requires PROPOSAL 10 or equivalent CPU-placement visibility for current profile interactions.
* Should follow PROPOSAL 13.

EDIT REQUEST FOR PATCH WRITER:
Add CPU placement awareness to `IrqAffinityProvider`. Replace `least_busy_cpu()` as the only target selector with a policy that accounts for focused workload CPUs, protected classes, housekeeping CPUs, topology, and reserved CPUs. Add tests proving IRQ suggestions do not target audio/compositor/protected CPUs and include structured placement evidence.

---

PROPOSAL 18: Remove unsafe fallback root task selection for apply-capable modes

STATUS: COMPLETED 2026-05-16 — made task fallback selection mode-aware, classed fallback roots as `Unknown`, blocked fallback mutable targets in apply-capable modes, added `target_snapshot_missing` denials, and covered nice/ionice/uclamp/cgroup provider behavior with tests.

PRIORITY: HIGH
When active task snapshots are missing, target selection manufactures a mutable helper-class root task, which is dangerous for autonomous mutation.

CURRENT STATE:
`mutable_task_targets_for_observation()` and `mutable_task_snapshots_for_observation()` use `fallback_root_snapshot(observation)` when `observation.active_tasks` is empty. 
`fallback_root_snapshot()` builds an `ActiveTaskSnapshot` using `target_root_pid`, sets `tid = root_pid`, `process_pid = root_pid`, and assigns `class: TaskClass::Helper`. 
Protected-task filtering then sees the fallback as `Helper`, not as `Unknown`, so process-local providers can target it in apply-capable paths.

PROPOSED CHANGE:
Replace fallback behavior with explicit mode-sensitive behavior:

* In suggest mode, fallback root may be used only for display/suggestions and must add evidence/deny message: `target_selection_fallback_root`.
* In apply modes, empty `active_tasks` must return no mutable targets.
* Add `CandidateDenyReason::TargetSnapshotMissing` or provider deny reason.
* `fallback_root_snapshot()` must classify fallback as `Unknown` unless the workload identity has a validated class.

AFFECTED SCOPE:

* `stutter/src/autotune/target_selection.rs`
* `stutter/src/autotune/providers/nice.rs`
* `stutter/src/autotune/providers/ioprio.rs`
* `stutter/src/autotune/providers/uclamp.rs`
* `stutter/src/autotune/providers/cgroup.rs`
* `stutter/src/autotune/planner.rs`
* tests
  Medium safety change.

DEPENDENCIES:

* Should be implemented before broad medium-risk apply is used.
* Related to PROPOSAL 19.

EDIT REQUEST FOR PATCH WRITER:
Remove apply-capable fallback targeting from `target_selection.rs`. When `active_tasks` is empty, process-local providers must not produce apply-eligible targets in apply modes. Classify fallback roots as unknown unless explicitly validated. Add tests proving nice/ionice/uclamp/cgroup providers emit no apply-eligible candidates when active task snapshots are missing.

---

PROPOSAL 19: Add target identity revalidation immediately before apply

STATUS: COMPLETED 2026-05-16 — added procfs-backed target identity revalidation to the privileged apply path, enumerated process-local candidate targets, checked missing TID/process PID/starttime/comm before executor creation, and added fake-proc tests for valid and stale identities.

PRIORITY: CRITICAL
A candidate selected from one observation must not mutate a PID/TID that has exited and been reused.

CURRENT STATE:
`target_selection.rs` converts `ActiveTaskSnapshot` into `TaskIdentity` with `tid`, `process_pid`, `comm`, and `starttime_ticks` from task/process snapshot fields. 
`LiveExperimentManager` starts experiments by applying the selected candidate later through `RuntimeLiveExperimentActionExecutor`, using a candidate cloned from planning time. 
`PrivilegedActionService::validate_candidate_plan_request()` validates plan age, descriptor match, objective match, and evidence count, but it does not re-read `/proc/<tid>/stat` to confirm all target identities are still the same before apply. 

PROPOSED CHANGE:
Add target identity revalidation to the privileged apply path:

* For every candidate containing process/task targets, enumerate `TaskIdentity` targets.
* Re-read `/proc/<tid>/stat` or equivalent under configured proc root.
* Confirm `starttime_ticks` matches if provided.
* Confirm process PID matches if provided.
* Confirm `comm` mismatch is either rejected or downgraded depending on policy.
* Reject the whole candidate if any target is stale, unless the action supports partial safe apply and policy explicitly allows it.

Add:

```rust
pub enum TargetRevalidationError {
    MissingTid,
    StarttimeMismatch,
    ProcessPidMismatch,
    CommMismatch,
}
```

AFFECTED SCOPE:

* `stutter/src/daemon/privilege.rs`
* `stutter/src/actions/mod.rs`
* `stutter/src/actions/nice.rs`
* `stutter/src/actions/ioprio.rs`
* `stutter/src/actions/uclamp.rs`
* `stutter/src/actions/cgroup.rs`
* `stutter/src/actions/cpu_affinity.rs`
* `stutter/src/procfs_utils.rs` or new module
* tests with fake proc roots
  Medium-large apply safety change.

DEPENDENCIES:

* Should follow PROPOSAL 18.
* Required before medium-risk process-local apply is default-trustworthy.

EDIT REQUEST FOR PATCH WRITER:
Add pre-apply task identity revalidation to the privileged action service. Before applying any candidate with task targets, re-read procfs and verify each target’s TID/process/starttime identity still matches the selected observation. Reject stale or reused targets with stable error codes. Add fake-proc tests for missing TID, reused TID, comm mismatch, and valid target.

---

PROPOSAL 20: Convert `InProcessPrivilegedActionService` into a real IPC-backed privileged worker

STATUS: COMPLETED 2026-05-16 — added a JSON-over-Unix-socket privileged worker command with 0600 socket permissions, a socket client implementing `PrivilegedActionService`, worker-side allowlist checks, medium-risk live-experiment service injection, unsafe in-process dev gating, docs, and socket integration tests.

PRIORITY: HIGH
The privilege boundary is currently an in-process abstraction; a full system-wide tuner needs an actual separated privileged mutator.

CURRENT STATE:
`daemon/privilege.rs` defines `PrivilegedActionService` with `dry_run_candidate`, `apply_candidate`, and `rollback`. It also defines roles, transports, operations, request authorization, and an allowlist. 
The only concrete service implementation is `InProcessPrivilegedActionService`, which directly calls `executor_for_candidate()`, `executor.dry_run()`, `executor.apply_with_audit()`, and `executor.rollback()`. 
`LiveExperimentManager` instantiates `InProcessPrivilegedActionService` directly for medium-risk apply and rollback. 

PROPOSED CHANGE:
Add an IPC-backed worker:

* Unix socket transport for local control plane → privileged worker.
* Request/response JSON or bincode schema.
* Authentication/authorization token or filesystem permission model.
* Worker process mode: `stutter privileged-worker --socket <path>`.
* Control-plane client implementing `PrivilegedActionService`.
* In-process service allowed only for tests and explicit unsafe dev mode.

Update `LiveExperimentManagerInput` to receive `Box<dyn PrivilegedActionService>` or a service handle instead of constructing `InProcessPrivilegedActionService` internally.

AFFECTED SCOPE:

* `stutter/src/daemon/privilege.rs`
* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/cli.rs`
* `stutter/src/commands/daemon.rs`
* `stutter/src/agent.rs`
* `contrib/openrc/stutter`
* `contrib/systemd/*.service` if present
* docs/install/safety/daemon contract
  Large architectural change.

DEPENDENCIES:

* Should follow PROPOSAL 19.
* Required before autonomous system-wide apply is acceptable.

EDIT REQUEST FOR PATCH WRITER:
Implement a real privileged worker process over a local Unix socket. Replace direct construction of `InProcessPrivilegedActionService` in live experiment code with dependency injection. Keep in-process service only for tests. Add command-line worker mode, IPC request/response schema, authentication/authorization checks, audit logging, and integration tests using a temporary Unix socket.

---

PROPOSAL 21: Audit privileged boundary decisions, not only action execution

STATUS: COMPLETED 2026-05-16 — added `PrivilegeAuditSink`, durable boundary audit events for worker allowlist decisions, validation allow/deny, stale/descriptor/evidence failures, apply/rollback start/completion/failure, and tests covering stale plans, descriptor mismatch, missing evidence, and successful worker apply/rollback audit records.

PRIORITY: HIGH
The privilege module defines audit event helpers, but the service path does not audit allow/deny decisions at the boundary.

CURRENT STATE:
`privileged_operation_audit_event()` builds an `AuditEvent` with command `"daemon_privilege"` and stable privilege action IDs. 
`InProcessPrivilegedActionService::dry_run_candidate()`, `apply_candidate()`, and `rollback()` validate the candidate and execute the action, but the shown implementation does not append a privilege-boundary audit event for request allowed/denied decisions. 
Action execution itself uses audited action runners, but that is not the same as auditing the boundary request and authorization decision.

PROPOSED CHANGE:
Add boundary audit writes for:

* privilege request received
* allowlist decision
* policy validation result
* stale plan rejection
* descriptor mismatch
* objective mismatch
* missing evidence
* apply started
* apply completed
* rollback requested
* rollback completed
* rollback failed

Add:

```rust
pub struct PrivilegeAuditSink { ... }
```

and pass it into `PrivilegedActionService` implementations.

AFFECTED SCOPE:

* `stutter/src/daemon/privilege.rs`
* `stutter/src/audit.rs`
* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/actions/runner.rs`
* tests for audit output
  Medium privilege/audit change.

DEPENDENCIES:

* Should be implemented before PROPOSAL 20 or as part of it.
* Required before remote/local privileged worker is trusted.

EDIT REQUEST FOR PATCH WRITER:
Add explicit audit logging for every privilege boundary decision in `daemon/privilege.rs`. Boundary audit must record allow/deny, reason code, caller role, transport, operation, policy intent, action ID, and error category. Add tests proving denied stale plans, descriptor mismatches, missing evidence, and successful apply/rollback all write privilege audit events.

---

PROPOSAL 22: Replace fake soak harness with scenario-driven live planner/controller simulation

STATUS: COMPLETED 2026-05-16 — replaced counter-only soak behavior with JSON scenario ticks that drive the online runtime simulation, added per-scenario reports and safety assertions, expanded soak fixtures to 12 scenarios including high-risk/manual-only cases, and kept CLI/acceptance compatibility through the existing soak command surface.

PRIORITY: HIGH
The current soak harness tests synthetic counters, not real planner/controller safety behavior.

CURRENT STATE:
`DaemonSoakProfile` only has `ObserveOnly` and `ApplyLowRiskFake`. 
`run_fake_daemon_soak()` simulates ticks, memory/disk/history growth, fake action counts, and fake rollback counts; it does not run actual planner proposals, controller decisions, live experiment transitions, active-config matching, target selection, privilege validation, or provider logic. 

PROPOSED CHANGE:
Create scenario-driven soak tests:

```rust
pub struct SoakScenario {
    pub name: String,
    pub ticks: Vec<SoakTick>,
    pub assertions: Vec<SoakAssertion>,
}
```

Required scenarios:

* game → browser → game
* compile background while browser foreground
* recording + game
* media playback + compile
* VM load + desktop interaction
* thermal degradation during experiment
* target disappears during experiment
* external mutation while kept action is active
* repeated candidate cooldown
* low data quality burst
* high-risk suggestion in suggest mode
* high-risk candidate in apply mode denied

Assertions:

* one active experiment maximum
* no high-risk autonomous apply
* no apply during low data quality
* no protected task mutation
* rollback token exists before apply
* shutdown restores active actions
* cooldown respected
* focus flapping does not cause action flapping

AFFECTED SCOPE:

* `stutter/src/daemon/soak.rs`
* `stutter/src/autotune/simulation.rs`
* `stutter/src/autotune/replay.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/live_experiment.rs`
* new `testdata/autotune/soak/*.json`
  Large test harness expansion.

DEPENDENCIES:

* Should follow PROPOSAL 10, PROPOSAL 13, PROPOSAL 18, and PROPOSAL 19.
* Required before medium-risk apply is made a normal user workflow.

EDIT REQUEST FOR PATCH WRITER:
Replace the fake soak harness with scenario-driven planner/controller simulation. Add JSON soak scenarios that feed observations through provider registry, planner, controller, and live experiment manager using fake executors. Assert no unsafe apply, correct rollback behavior, cooldowns, protected-task safety, and high-risk manual-only behavior.

---

PROPOSAL 23: Rename and generalize `LiveLowRiskExperiment`

STATUS: COMPLETED 2026-05-16 — renamed live experiment and active autotune registry types, stored actual mode/safety class on live experiment state, propagated mode/safety through daemon experiment and rollback state, journal metadata, daemon status text, and history decision summaries, and added a medium-risk state regression test.

PRIORITY: MEDIUM
The live experiment manager now handles medium-risk apply through the privileged service, but its primary state type still encodes “low risk” in its name.

CURRENT STATE:
`live_experiment.rs` defines `LiveLowRiskExperiment` with experiment ID, candidate, baseline score/signals, applied time, washout/measurement deadlines, and rollback token. 
`LiveExperimentManager` can start medium-risk candidates when `input.mode == DaemonMode::ApplyMediumRisk`; it uses `InProcessPrivilegedActionService` for medium-risk apply and the legacy low-risk path otherwise. 
`validate_start_candidate()` explicitly handles both `ApplyLowRisk` and `ApplyMediumRisk`. 

PROPOSED CHANGE:
Rename:

* `LiveLowRiskExperiment` → `LiveExperiment`
* `ActiveLowRiskActionRegistry` references, if still generic, to `ActiveAutotuneActionRegistry`
* low-risk-specific variable names where they now include medium-risk candidates

Add field:

```rust
pub safety_class: SafetyClass
pub mode: DaemonMode
```

to the live experiment state. Ensure daemon status, rollback state, journal metadata, and history output include actual mode and safety class.

AFFECTED SCOPE:

* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/autotune/shutdown.rs`
* `stutter/src/autotune/runtime.rs`
* `stutter/src/daemon/state.rs`
* `stutter/src/autotune/history.rs`
* tests
  Mostly mechanical but medium ripple.

DEPENDENCIES:

* Should follow medium-risk apply path stabilization.
* Helps PROPOSAL 22 and PROPOSAL 24.

EDIT REQUEST FOR PATCH WRITER:
Rename live experiment state from low-risk-specific names to generic names. Store safety class and daemon mode in the experiment state. Update daemon status, rollback state, journal records, history records, and tests so medium-risk experiments are not represented as low-risk experiments.

---

PROPOSAL 24: Add multi-action kept-state model with explicit conflict resolution

PRIORITY: HIGH
A full system-wide tuner eventually needs multiple compatible kept actions, but current kept-state handling appears to model only one current kept candidate.

CURRENT STATE:
Planner checks one kept action through `active_profile_state.current.as_ref()` and denies candidates that conflict with that single kept candidate. 
`LiveExperimentManager` receives `ActiveProfileState` and writes daemon rollback state for the current experiment, but the shown planner interaction does not support a set of compatible kept actions.
`CandidateAction::conflict_group()` exists, and `conflicts_with()` compares conflict groups. 

PROPOSED CHANGE:
Replace single kept action state with:

```rust
pub struct KeptActionSet {
    pub actions: BTreeMap<ActionConflictGroup, KeptCandidateState>,
}
```

Rules:

* One kept action per conflict group by default.
* Compatible conflict groups may coexist.
* New candidate replaces an existing kept action only through explicit replace/rollback sequence.
* Shutdown must restore all non-persistent kept actions.
* Status must list all kept actions.
* Planner must check candidate against every kept action.

AFFECTED SCOPE:

* `stutter/src/autotune/kept.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/autotune/shutdown.rs`
* `stutter/src/daemon/state.rs`
* `stutter/src/autotune/status.rs`
* tests
  Large state model change.

DEPENDENCIES:

* Should follow PROPOSAL 23.
* Needed before combining CPU affinity + process-local priority + cgroup kept actions.

EDIT REQUEST FOR PATCH WRITER:
Replace single kept-action state with a conflict-group-indexed kept action set. Update planner conflict checks, live experiment keep/revert logic, shutdown restore, daemon status, and history to support multiple compatible kept actions while still preventing conflicting actions from stacking.

---

PROPOSAL 25: Add real external mutation recovery workflow

PRIORITY: HIGH
Planner can detect external mutation, but the system needs a recovery path that tells the daemon whether to restore, resync, or abandon state.

CURRENT STATE:
Planner adds `ExternalMutationDetected` when an active experiment’s candidate conflicts with the proposal and the active candidate’s `matches_active_config()` returns `Differs`. It adds `KeptActionNoLongerActive` when a kept candidate differs from live state. 
No complete recovery workflow is visible in the planner code: it only denies new candidates and emits messages such as “restore or resync before planning new candidates.” 

PROPOSED CHANGE:
Add daemon recovery decisions:

* `RestoreExpectedState`
* `AcceptExternalMutationAndResync`
* `AbandonKeptAction`
* `FaultRequireManualRestore`

Add config:

```toml
external_mutation_policy = "fault" | "restore" | "resync"
```

Default:

* active experiment mutation → rollback/fault
* kept action mutation → observe-only/fault unless explicit resync configured

Add command:

```bash
stutter daemon resync-state --dry-run
stutter daemon resync-state
```

AFFECTED SCOPE:

* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/autotune/kept.rs`
* `stutter/src/daemon/state.rs`
* `stutter/src/commands/daemon.rs`
* `stutter/src/cli.rs`
* docs
  Medium-large behavior addition.

DEPENDENCIES:

* Requires PROPOSAL 10 for CPU-affinity external mutation.
* Should follow PROPOSAL 24.

EDIT REQUEST FOR PATCH WRITER:
Implement an explicit external mutation recovery workflow. Planner denial must lead to a daemon state transition: restore, resync, abandon kept action, or fault. Add config and CLI commands for safe resync. Add tests for active experiment mutation, kept action mutation, restore success, restore failure, and manual resync.

---

PROPOSAL 26: Make high-risk manual suggestions produce dry-run evidence without enabling apply

PRIORITY: MEDIUM
High-risk candidates are manual-only, but users still need safe dry-run diagnostics for why a high-risk suggestion exists and what it would touch.

CURRENT STATE:
Planner marks high-risk/system-adjacent candidates with `ManualOnlyHighRisk` before dry-run. 
`dry_run_candidate_if_still_eligible()` skips dry-run when mode is `Suggest` and the candidate is high-risk/system-adjacent. 
High-risk providers emit evidence, but dry-run affected state is not collected in normal suggest mode.

PROPOSED CHANGE:
Add a separate safe high-risk dry-run mode:

* `suggest`: no high-risk dry-run by default
* `suggest --high-risk-dry-run`: run high-risk dry-run only, never apply
* dry-run must use policy intent `DryRun`
* dry-run output must include affected scope and rollback availability
* apply command must remain absent/manual-blocked

Add planner field:

```rust
pub high_risk_dry_run: bool
```

or policy/config equivalent.

AFFECTED SCOPE:

* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/candidate.rs`
* `stutter/src/cli.rs`
* `stutter/src/commands/autotune.rs`
* `stutter/src/autotune/status.rs`
* docs
  Medium CLI/planner addition.

DEPENDENCIES:

* Must keep PROPOSAL 20 and high-risk apply guards intact.
* Should follow PROPOSAL 21 audit improvements.

EDIT REQUEST FOR PATCH WRITER:
Add an explicit high-risk dry-run-only suggestion mode. Keep high-risk candidates manual-only and non-applyable, but allow users to request audited dry-run diagnostics for high-risk candidates. Update planner, CLI, status output, and tests so high-risk dry-run never produces a live `StartExperiment`.

---

PROPOSAL 27: Add confidence calibration per provider family

PRIORITY: MEDIUM
Provider confidence currently uses local completeness formulas, but there is no central calibration to make scores comparable across action families.

CURRENT STATE:
`CandidateProposal` includes `confidence`, and planner uses policy thresholds and ranking by confidence.
IRQ confidence is `situation.confidence * completeness`; GPU confidence is `situation.confidence * completeness`; CPU power confidence is also `situation.confidence * completeness`.
These formulas use different completeness dimensions and therefore are not calibrated across provider families.

PROPOSED CHANGE:
Add:

```rust
pub struct ProviderConfidenceCalibration {
    pub family: String,
    pub min_required_signals: Vec<String>,
    pub direct_signal_weight: f32,
    pub inferred_signal_weight: f32,
    pub max_without_direct_signal: f32,
    pub max_without_active_config: f32,
}
```

Add per-family calibration defaults:

* CPU affinity: high if dry-run affects targets and focus confidence high
* nice/ionice/uclamp/cgroup: require active task snapshots
* IRQ: cap if no stable IRQ identity or no current mask
* GPU: cap if no focused render node on multi-GPU
* CPU power: cap if no AC/battery state
* VM: cap if no direct memory/swap/writeback signal

Apply calibration centrally in planner or provider registry after proposal creation.

AFFECTED SCOPE:

* `stutter/src/autotune/providers/mod.rs`
* `stutter/src/autotune/planner.rs`
* all provider files
* `stutter/src/daemon/config.rs`
* docs/config
  Medium provider-policy change.

DEPENDENCIES:

* Should follow PROPOSAL 13, PROPOSAL 14, and PROPOSAL 15.

EDIT REQUEST FOR PATCH WRITER:
Add centralized provider confidence calibration. Do not let providers return uncalibrated confidence directly into planner ranking. Implement per-family caps and required signals. Add tests proving missing focused GPU, missing AC power, missing active config, and missing IRQ identity cap confidence below apply thresholds.

---

PROPOSAL 28: Add hardware allowlists for CPU/GPU/IRQ/VM system-adjacent suggestions

PRIORITY: HIGH
System-wide suggestions must be constrained to user-approved devices and knobs before the project moves toward automated system-wide tuning.

CURRENT STATE:
`DaemonPolicy` has system-wide suggestion/apply flags, high-risk flags, enabled/denied action families, and cgroup targets. 
High-risk providers can emit IRQ, CPU power, GPU power, and VM knob candidates using inventory and active config.
There is no cited equivalent of `cgroup_targets` for allowed DRM cards, CPU policies, IRQ devices, or sysctl knobs.

PROPOSED CHANGE:
Add policy config:

```toml
[system_wide_allowlist]
cpu_policies = ["policy0", "policy1"]
gpu_cards = ["card0"]
gpu_pci_ids = ["1002:*"]
irq_devices = ["amdgpu", "xhci_hcd"]
vm_knobs = ["proc/sys/vm/swappiness"]
```

Planner/provider behavior:

* High-risk providers must not emit candidates outside allowlist unless in diagnostic-only mode.
* Status must report deny reason `SystemWideTargetNotAllowlisted`.
* Empty allowlist means no system-wide targets are allowed by default for apply; suggestions may be diagnostic-only depending on config.

AFFECTED SCOPE:

* `stutter/src/daemon/config.rs`
* `stutter/src/daemon/policy.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/providers/irq_affinity.rs`
* `stutter/src/autotune/providers/cpu_power.rs`
* `stutter/src/autotune/providers/gpu_power.rs`
* `stutter/src/autotune/providers/vm_knob.rs`
* docs/config
  Medium policy/provider change.

DEPENDENCIES:

* Should follow PROPOSAL 14, PROPOSAL 15, and PROPOSAL 16.
* Required before any future high-risk apply work.

EDIT REQUEST FOR PATCH WRITER:
Add system-wide target allowlists for CPU policies, GPUs, IRQ devices, and VM knobs. Enforce them in high-risk providers or planner. Add structured denial reasons and tests proving non-allowlisted cards, policies, IRQs, and sysctls are not suggested or applied.

---

PROPOSAL 29: Add rollback verification after every rollback operation

PRIORITY: HIGH
Rollback success must be verified against active config, not trusted only because the rollback function returned `Ok`.

CURRENT STATE:
`LiveExperimentManager` stores a rollback token and calls the privileged service’s `rollback()` during rollback paths.
`InProcessPrivilegedActionService::rollback()` checks policy, calls `executor.rollback(&request.token)`, and returns affected task count. 
No cited rollback path re-collects `ActiveConfigSnapshot` and verifies the system returned to the expected baseline state.

PROPOSED CHANGE:
Capture pre-apply baseline active config for the candidate conflict group. Store it with live experiment state and rollback token. After rollback:

* Recollect active config.
* Compare conflict-group-relevant state to baseline.
* If rollback did not restore expected state, enter fault state and expose manual restore command.
* Audit rollback verification result.

Add:

```rust
pub struct RollbackVerification {
    pub verified: bool,
    pub expected: String,
    pub actual: String,
    pub reason_code: String,
}
```

AFFECTED SCOPE:

* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/daemon/privilege.rs`
* `stutter/src/autotune/active_config.rs`
* `stutter/src/autotune/shutdown.rs`
* `stutter/src/actions/*`
* tests
  Large safety change.

DEPENDENCIES:

* Requires PROPOSAL 10 and PROPOSAL 13.
* Should precede any autonomous medium-risk default workflow.

EDIT REQUEST FOR PATCH WRITER:
Add rollback verification. Capture conflict-group-specific active config before apply, verify active config after rollback, and fault if rollback does not restore expected state. Add audit events and tests for successful rollback, incomplete rollback, missing target after rollback, and rollback verification unavailable.

---

PROPOSAL 30: Add build/test/CI gate for the full watcher safety matrix

PRIORITY: HIGH
The project now has enough safety-sensitive architecture that CI must enforce the invariants automatically.

CURRENT STATE:
The workspace has Rust crates `stutter`, `stutter-common`, and `stutter-ebpf`, with default members `stutter` and `stutter-common`. 
The codebase contains planner, privilege, provider, objective, rolling-window, and soak tests in source modules, but the review environment did not verify a full build/test run.
Safety invariants include high-risk apply disabled, medium-risk unlock required, manual-only high-risk suggestions, no protected task selection, and policy gates in planner. These are distributed across `planner.rs`, `policy.rs`, `target_selection.rs`, `privilege.rs`, and provider modules.

PROPOSED CHANGE:
Add CI jobs:

* `cargo fmt --all --check`
* `cargo build --all-targets`
* `cargo clippy --all-targets -- -D warnings`
* `cargo test --all-targets`
* focused safety tests:

  * planner golden fixtures
  * high-risk apply disabled
  * protected task selection
  * privilege boundary
  * rollback verification
  * soak scenarios

Add a script:

```bash
scripts/check-autotune-safety.sh
```

that runs only the safety matrix tests for local iteration.

AFFECTED SCOPE:

* `.github/workflows/*.yml`
* `scripts/check-autotune-safety.sh`
* test modules across `autotune`, `daemon`, `actions`
* docs contributing/test instructions
  Medium repository/CI change.

DEPENDENCIES:

* Should be maintained continuously.
* Add new safety tests from proposals as they land.

EDIT REQUEST FOR PATCH WRITER:
Add CI and local scripts that enforce formatting, build, clippy, full tests, and an explicit autotune safety matrix. Include tests for high-risk apply disabled, protected task exclusion, medium-risk unlock, privilege boundary denial, planner no-op detection, rollback verification, and soak scenarios.

---

PROPOSAL 31: Decompose large architecture hubs after safety gates are stable

PRIORITY: MEDIUM
Large files are now the main maintainability risk and will slow future full-system tuning work.

CURRENT STATE:
`candidate.rs` owns candidate enum, plan file schema, executable plan schema, evidence, candidate-plan trait, all plan structs, CPU-affinity plan metadata, and much more.
`planner.rs` owns deny reasons, evaluation DTOs, planner summaries, sorting, grouping, provider input construction, static evaluation, active-config checks, dry-run gating, and tests.
`live_experiment.rs` owns experiment state, privileged apply, rollback, journal side effects, controller state mutation, keep/revert decisions, deadline computation, objective comparison, and history contexts.

PROPOSED CHANGE:
Split modules:

`candidate.rs` into:

* `candidate/mod.rs`
* `candidate/action.rs`
* `candidate/plan.rs`
* `candidate/evidence.rs`
* `candidate/plan_file.rs`
* `candidate/executable.rs`
* `candidate/manual_commands.rs`

`planner.rs` into:

* `planner/mod.rs`
* `planner/deny.rs`
* `planner/evaluation.rs`
* `planner/summary.rs`
* `planner/sort.rs`
* `planner/static_gates.rs`
* `planner/dry_run.rs`

`live_experiment.rs` into:

* `live_experiment/mod.rs`
* `live_experiment/state.rs`
* `live_experiment/apply.rs`
* `live_experiment/rollback.rs`
* `live_experiment/journal.rs`
* `live_experiment/keep_revert.rs`

No behavior changes in the split PR. Move tests with their modules.

AFFECTED SCOPE:

* `stutter/src/autotune/candidate.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/live_experiment.rs`
* imports across autotune/provider/runtime/status modules
* tests
  Large mechanical refactor.

DEPENDENCIES:

* Should happen after PROPOSAL 10, PROPOSAL 18, and PROPOSAL 19 to avoid merge conflicts.
* Should happen before adding high-risk apply implementation.

EDIT REQUEST FOR PATCH WRITER:
Perform a behavior-preserving module split of `candidate.rs`, `planner.rs`, and `live_experiment.rs`. Keep public APIs stable through `mod.rs` re-exports. Do not change runtime behavior. Move tests into the closest new module. Run full fmt/build/clippy/test after the split.

---

PROPOSAL 32: Add automated workload policy validation and linting

PRIORITY: MEDIUM
Configurable workload policy is powerful, but invalid policy can accidentally allow dangerous autonomous actions.

CURRENT STATE:
Planner uses `workload_policy.rule_for(observation.primary_situation)` and enforces `allows_candidate`, `allows_autonomous_candidate`, and `allows_objective`. 
`DaemonPolicy` has enabled/denied action families, allowed scopes, confidence config, and system-wide flags. 
System-wide/high-risk action families can be suggested but must remain blocked for autonomous apply.

PROPOSED CHANGE:
Add policy linter:

```rust
pub struct WorkloadPolicyLint {
    pub severity: LintSeverity,
    pub reason_code: String,
    pub message: String,
}
```

Lint rules:

* high-risk families must not appear in `autonomous_families` while high-risk apply disabled
* system-wide families must not be autonomous unless system-wide apply enabled
* objective must match family capability
* empty autonomous list must be explicit, not accidental
* denied family must not also be autonomous
* apply-low-risk presets must not enable medium/high-risk autonomous families

Expose:

```bash
stutter daemon policy-lint
stutter daemon policy-lint --json
```

AFFECTED SCOPE:

* `stutter/src/autotune/workload_policy.rs`
* `stutter/src/daemon/policy.rs`
* `stutter/src/commands/daemon.rs`
* `stutter/src/cli.rs`
* config docs/tests
  Medium config safety addition.

DEPENDENCIES:

* Should follow system-wide allowlist work from PROPOSAL 28.
* Helps before user-editable full-system policy.

EDIT REQUEST FOR PATCH WRITER:
Implement workload policy linting. Add CLI and JSON output. Enforce lints in tests for default policies and fail config loading on critical policy contradictions. Ensure high-risk/system-wide families cannot become autonomous through config while high-risk apply remains disabled.

---

PROPOSAL 33: Add real end-to-end “autotune dry-run daemon” mode

PRIORITY: HIGH
Users need a mode that exercises the full watcher stack without mutating system state.

CURRENT STATE:
`DaemonMode` has `Observe`, `Suggest`, and apply modes. Policy allows `Suggest` when mode is not observe and system-wide suggestions are permitted depending on policy. 
Planner can produce evaluations and summaries. 
High-risk dry-run is skipped by default in suggest mode. 

PROPOSED CHANGE:
Add mode or flag:

```bash
stutter autotune --mode suggest --dry-run-all-safe
```

Behavior:

* Run provider registry.
* Run static gates.
* Run dry-run for candidates whose safety class and effect scope allow dry-run under policy.
* Never start experiments.
* Output candidate plan files, summaries, dry-run affected tasks, and deny reasons.
* Include high-risk dry-run only if explicitly requested per PROPOSAL 26.

AFFECTED SCOPE:

* `stutter/src/cli.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/runtime.rs`
* `stutter/src/autotune/status.rs`
* docs
  Medium feature addition.

DEPENDENCIES:

* Should follow PROPOSAL 26.
* Useful before medium-risk apply rollout.

EDIT REQUEST FOR PATCH WRITER:
Add an end-to-end dry-run daemon/suggest mode that exercises planning and safe dry-run without mutation. It must produce structured summaries and candidate plan files. It must never call live experiment start/apply. Add tests proving no rollback token is created and no action apply path runs.

---
