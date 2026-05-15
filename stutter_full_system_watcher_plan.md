# `stutter` full system watcher completion plan

Static review basis: uploaded `stutter-main.zip`, focused on `stutter/src/autotune`, `stutter/src/actions`, `stutter/src/focus`, `stutter/src/daemon`, `stutter/src/session`, `stutter/src/recorder`, `stutter/src/report`, and existing docs under `docs/`.

This plan assumes the target product is:

> An always-on Linux performance watcher that detects the currently focused workload, classifies the current performance situation, proposes safe tuning candidates, applies one reversible candidate at a time when policy allows it, verifies improvement, keeps or rolls back, remembers results per workload/environment, and eventually coordinates CPU affinity, nice, ionice, uclamp, cgroups, IRQ affinity, CPU power, GPU power, VM knobs, and scheduler-related configuration.

The repo already has a large amount of the hard plumbing: monitor events, focus grouping, scoring, action traits, rollback tokens, audit events, daemon policy, health/watchdog logic, startup recovery, action modules, and low-risk CPU-affinity autotune. The missing part is the middle: a general planner and action-provider layer that can safely connect observations to multiple action families.

---

## Current source architecture to preserve

These are the current boundaries that should stay mostly intact:

- `stutter/src/actions/*`: concrete mutating actions. Existing families: CPU affinity, nice, ionice, uclamp, cgroup placement, IRQ affinity, CPU power, GPU power, VM knobs.
- `stutter/src/actions/runner.rs`: generic action lifecycle: preflight, dry-run, apply, verify, rollback, audit, timeout handling.
- `stutter/src/autotune/*`: online controller, candidate selection, low-risk apply, comparison, quality, history, decision logs, startup recovery, shutdown rollback, simulation.
- `stutter/src/focus/*`: focus snapshot, process classification, focus groups, focus scoring, foreground-aware focus resolution.
- `stutter/src/daemon/*`: daemon config, policy, capabilities, health, watchdog, runtime state, lifecycle, privilege model, status/explain/soak/acceptance tests.
- `stutter/src/session/*`: monitor runtime, event bus, targets, output sinks, telemetry.
- `stutter/src/recorder/*` and `stutter/src/report/*`: durable artifact and reporting layer.

Important current constraints:

- Live `CandidateAction` is still effectively CPU-affinity-only plus fake test actions.
- `apply_low_risk` currently supports CPU-affinity candidates only.
- Medium/high daemon modes exist as policy labels, but live apply paths are intentionally blocked/incomplete.
- Focus and situation detection exist, but situation classification is still too coarse and partly derived from the selected focus group rather than full live evidence.
- `history_situation()` currently compresses browser, media, recording, VM, and compile sub-situations into `CompileLoad`, which is not acceptable for a full watcher.

---

## Global definition of done

The project is “fully completed” for the described goal when all of these are true:

1. `stutter daemon` or `stutter autotune` can run continuously in observe/suggest/apply modes.
2. The daemon can answer: “what is the system focusing on right now?” with confidence, evidence, and safety warnings.
3. The daemon can classify the current situation into workload-aware categories, not just “game or unknown”.
4. Each action family has a `CandidateProvider` that can propose candidates with evidence, safety class, required mode, rollback requirement, capability requirements, conflict group, cooldown key, and verification objective.
5. Suggestions are available for every supported action family before autonomous apply is allowed.
6. Apply modes are strict:
   - `observe`: never suggests/apply as a controller decision.
   - `suggest`: never mutates.
   - `apply-low-risk`: local/process-tree, reversible, low-risk only.
   - `apply-medium-risk`: explicit opt-in, reversible medium-risk, still local/process/cgroup-scoped unless explicitly broadened.
   - `apply-high-risk`: explicit opt-in, system-wide/high-risk, never default, no remote-by-default.
7. The controller applies at most one new experiment at a time.
8. Every applied candidate has a rollback token before or immediately after mutation, is journaled, and is recoverable after crash/startup.
9. Health, data-quality, focus stability, target identity, capability, and cooldown gates can block action and explain why.
10. Every “keep” decision is based on a workload-specific objective and a comparable measurement window.
11. Every “revert” path works under tests, including target exit, identity mismatch, low quality, regression, health degradation, suspend/resume, and crash recovery.
12. The service has soak tests proving that the daemon does not leak state, spam actions, fill disk, or flap between focus targets.
13. Docs and CLI output never claim a mode can apply an action before policy and runtime both support it.

---

# Milestone 1 — Reliable observer

Goal: before changing more system state, make `stutter` excellent at deciding what the machine is focused on, what situation is happening, and why actions are blocked or suggested.

## 1.1 Freeze and document the existing module contract

Files:

- `docs/DAEMON_CONTRACT.md`
- `docs/AUTOTUNE_ARCHITECTURE.md`
- `docs/AUTOTUNE_IMPLEMENTATION_CHECKLIST.md`
- `docs/ROADMAP.md`
- new: `docs/FULL_SYSTEM_WATCHER_ARCHITECTURE.md`

Steps:

1. Add a new architecture doc with the final loop:
   - collect monitor events;
   - update rolling windows;
   - build focus snapshot;
   - resolve focus;
   - classify situation;
   - evaluate data quality and health;
   - ask candidate providers for candidates;
   - policy-filter candidates;
   - rank candidates;
   - suggest or apply one candidate;
   - measure;
   - keep/revert;
   - cooldown.
2. Explicitly define the boundary between:
   - observation;
   - diagnosis;
   - candidate planning;
   - action execution;
   - verification;
   - rollback.
3. Add a “no direct mutation from planner” rule. The planner may return candidates only; mutation still goes through `TuningAction` + `actions/runner.rs` + `DaemonPolicy`.
4. Add a “suggest first” rule for every new provider.
5. Add a compatibility table mapping action families to modes.

Tests/checks:

- Documentation-only PR can still run `cargo test` to ensure no doctest issues if doctests are enabled.

Done when:

- A new contributor can tell where to add an observer, where to add a provider, and where mutation is allowed.

Implementation status:

- [x] 2026-05-15: Added `docs/FULL_SYSTEM_WATCHER_ARCHITECTURE.md` with the final watcher loop, boundary rules, no-direct-mutation/suggest-first rules, and action family/mode compatibility table.

## 1.2 Unify `SituationKind`

Current problem:

- `stutter/src/autotune/state.rs` has the richer `SituationKind`.
- `stutter/src/autotune/history.rs` has a smaller duplicated `SituationKind`.
- `stutter/src/autotune/runtime.rs::history_situation()` maps browser/media/recording/VM cases to `CompileLoad`, losing meaning.

Files:

- `stutter/src/autotune/state.rs`
- `stutter/src/autotune/history.rs`
- `stutter/src/autotune/decision_log.rs`
- `stutter/src/autotune/human_output.rs`
- `stutter/src/autotune/prometheus_metrics.rs`
- `stutter/src/autotune/report_overlay.rs`
- `stutter/src/autotune/runtime.rs`
- tests in the same modules

Steps:

1. Move `SituationKind` into a single source-of-truth module, preferably new file:
   - `stutter/src/autotune/situation.rs`.
2. Re-export it from `stutter/src/autotune/mod.rs` or `state.rs` for compatibility.
3. Remove the separate enum in `history.rs`, or make it a type alias/re-export.
4. Update JSON serialization to preserve existing names for old variants.
5. Add missing variants to history/decision-log/human/prometheus/report overlay:
   - `BrowserFocused`
   - `BrowserCpuPressure`
   - `BrowserGpuVideo`
   - `BrowserIoPressure`
   - `CompileCpuBound`
   - `CompileLinkerPressure`
   - `MediaPlayback`
   - `Recording`
   - `VirtualMachineLoad`
6. Replace `history_situation()` with either identity conversion or a lossy conversion that explicitly uses `Other(String)` if old schema compatibility is required.
7. Add tests that each `SituationKind` variant round-trips through history JSON and decision JSONL.
8. Add a regression test proving `BrowserFocused` does not serialize as `CompileLoad`.

Done when:

- No situation variant is silently collapsed into the wrong workload class.

Implementation status:

- [x] 2026-05-15: Added `stutter/src/autotune/situation.rs` as the shared `SituationKind` source of truth, re-exported it from state/history, widened decision labels, removed the history collapse path, and added history/decision JSON round-trip regression coverage including `BrowserFocused`.

## 1.3 Add a real situation classifier

Current state:

- Focus resolver maps focus group kind to base situations.
- Runtime candidate ranking has hardcoded `candidate_situation_rank()` for a few game/compositor/CPU cases.
- Existing diagnosis entries and frame/GPU/IRQ/IO signals are not yet first-class inputs to a situation classifier.

Files:

- new: `stutter/src/autotune/situation.rs`
- `stutter/src/autotune/observation.rs`
- `stutter/src/autotune/runtime.rs`
- `stutter/src/diagnosis.rs`
- `stutter/src/scorer.rs`
- `stutter/src/recorder/event_types.rs`
- `stutter/src/session_events.rs`

New types:

```rust
pub struct SituationClassification {
    pub primary: SituationKind,
    pub secondary: Vec<SituationKind>,
    pub confidence: f32,
    pub evidence: Vec<SituationEvidence>,
    pub blockers: Vec<SituationBlocker>,
}

pub struct SituationEvidence {
    pub signal: String,
    pub value: String,
    pub weight: f32,
}

pub enum SituationBlocker {
    LowFocusConfidence,
    LowDataQuality,
    MissingFrameData,
    MissingGpuData,
    MissingIrqData,
    ThermalDegraded,
}
```

Steps:

1. Implement `classify_situation(observation_input) -> SituationClassification`.
2. Start with deterministic rules:
   - game focus + high runnable latency on game/render threads => `GameCpuSchedulerPressure`;
   - game focus + frame p99/max bad + GPU busy/high clocks if available => `GameGpuBound`;
   - compositor classified as pressured => `CompositorPressure`;
   - browser focus + high CPU score => `BrowserCpuPressure`;
   - browser focus + frame/video evidence => `BrowserGpuVideo`;
   - compile focus + high CPU samples => `CompileCpuBound`;
   - linker process detected or high write/read in linker-like process => `CompileLinkerPressure`;
   - high block I/O overlap => `IoPressure` or workload-specific `BrowserIoPressure`;
   - IRQ evidence near spikes => `IrqPressure`;
   - health says overheated/power-limited => `ThermalOrPowerLimit`;
   - recorder/OBS/pipewire capture focus => `Recording`;
   - media player focus => `MediaPlayback`;
   - VM process focus => `VirtualMachineLoad`;
   - no active target and low user activity => `Idle`;
   - else `Unknown`.
3. Use existing `recent_diagnoses: Vec<LiveDiagnosisEntry>` inside `AutotuneObservation` as evidence, not just display text.
4. Keep the classifier pure: no filesystem writes, no policy checks, no mutation.
5. Add a `SituationClassification` field to `AutotuneObservation` or replace `primary_situation` with a richer field while keeping serialization stable.
6. Add `reason_codes` for machine clients.

Tests:

- `game_focus_with_scheduler_spikes_classifies_game_cpu_scheduler_pressure`.
- `game_focus_with_bad_frames_and_gpu_evidence_classifies_game_gpu_bound`.
- `browser_focus_with_cpu_pressure_classifies_browser_cpu_pressure`.
- `compile_focus_with_linker_comm_classifies_compile_linker_pressure`.
- `thermal_degraded_overrides_performance_actions`.
- `low_quality_preserves_situation_but_marks_blocker`.
- `unknown_focus_does_not_invent_candidate_situation`.

Done when:

- Situation selection is no longer a simple focus-group mapping plus profile-name ranking.

Implementation status:

- [x] 2026-05-15: Added pure `classify_situation` with `SituationClassification`, evidence, blockers, reason codes, and deterministic diagnosis/focus/health rules with tests for game CPU, game GPU, browser CPU, linker pressure, low quality, and unknown focus behavior.

## 1.4 Enrich `AutotuneObservation`

Current fields are good but too narrow for a full watcher.

Files:

- `stutter/src/autotune/observation.rs`
- `stutter/src/autotune/runtime.rs::build_observation`
- `stutter/src/autotune/history.rs`
- `stutter/src/autotune/decision_log.rs`
- `stutter/src/autotune/prometheus_metrics.rs`
- `stutter/src/autotune/status.rs`
- `stutter/src/daemon/health.rs`
- `stutter/src/daemon/capabilities.rs`

Add fields gradually:

```rust
pub struct AutotuneObservation {
    // existing fields stay
    pub situation: SituationClassification,
    pub system_health: SystemHealthSnapshot,
    pub capabilities: DaemonCapabilities,
    pub topology_signature: Option<String>,
    pub workload_identity: Option<WorkloadIdentity>,
    pub protected_tasks: Vec<ProtectedTask>,
    pub active_config_snapshot: Option<ActiveConfigSnapshot>,
}
```

Steps:

1. Add a `WorkloadIdentity` struct:
   - root PID;
   - process starttime;
   - executable dev/ino;
   - cgroup path;
   - focus kind;
   - class distribution;
   - stable hash.
2. Add a `ProtectedTask` struct for compositor, audio, input, recorder, kernel helper-like processes, and critical realtime tasks.
3. Add an optional `ActiveConfigSnapshot` containing current affinity/nice/uclamp/cgroup/IRQ/CPU/GPU/VM state only when cheap and safe to read.
4. Update `Default` safely.
5. Update history and decision outputs with schema versioning.
6. Keep old fields like `primary_situation` for compatibility until all consumers migrate.

Tests:

- default observation blocks action.
- observation with health degraded blocks apply through policy context.
- workload identity changes when executable inode/starttime changes.
- protected tasks are present in observation but excluded from mutation candidates.

Done when:

- Candidate providers do not need to re-probe random global state to know basic context.

Implementation status:

- [x] 2026-05-15: Enriched `AutotuneObservation` with situation classification, system health, daemon capabilities, workload identity, protected tasks, topology/config placeholders, defaults, and runtime population from target snapshots.

## 1.5 Harden focus detection

Files:

- `stutter/src/focus/snapshot.rs`
- `stutter/src/focus/classify.rs`
- `stutter/src/focus/groups.rs`
- `stutter/src/focus/resolve.rs`
- `stutter/src/focus/score.rs`
- `stutter/src/foreground.rs`
- `stutter/src/commands/input.rs`
- config files under `stutter/src/config/*`

Steps:

1. Preserve existing confidence/switch-margin/cooldown logic in `FocusResolver`.
2. Add `FocusDecision` explanation fields if needed:
   - selected group;
   - runner-up group;
   - switch blocked by margin;
   - switch blocked by cooldown;
   - switch blocked by required polls;
   - safety warnings.
3. Add a `FocusProviderStatus` value to snapshots:
   - `Unavailable`;
   - `Stale`;
   - `AvailableRedacted`;
   - `AvailableWithTitle`.
4. Add Hyprland foreground provider implementation if desired:
   - currently docs say selector exists but unsupported;
   - keep title redaction default.
5. Add an input-activity signal from existing `commands/input.rs` concepts into focus scoring:
   - keyboard/mouse/gamepad recent activity;
   - idle detection;
   - active seat if possible.
6. Add protected process override:
   - pipewire/wireplumber/audio realtime tasks should never become deprioritized background unless explicitly configured;
   - compositor should be protected even when game is focused.
7. Make `Recording` focus sticky when OBS/recording tools are active, even if a game is foreground.
8. Add “foreground mismatch” evidence when foreground app does not match heuristic CPU-heavy app.

Tests:

- foreground app wins only when safe.
- foreground unknown app becomes conservative fallback, not broad system root.
- recording active prevents unsafe game-only tuning.
- compositor in focus group adds safety warning.
- critical realtime task warning blocks mutation.
- input idle state allows `Idle` only after stable no-focus interval.

Done when:

- The daemon can explain focus decisions well enough that wrong focus decisions are debuggable.

## 1.6 Make data-quality policy situation-aware

Files:

- `stutter/src/autotune/quality.rs`
- `stutter/src/autotune/comparison.rs`
- `stutter/src/autotune/measurement.rs`
- `stutter/src/autotune/baseline.rs`
- `stutter/src/autotune/runtime.rs`

Steps:

1. Keep current minimum intervals/samples/drop-counter checks.
2. Add per-situation frame requirements:
   - game frame-pacing candidates require frame data when comparing frame objectives;
   - compile/browser/desktop CPU candidates do not require frame data.
3. Add GPU-data quality status:
   - unavailable;
   - stale;
   - present;
   - inconsistent device.
4. Add IRQ-data quality status.
5. Add I/O data quality status.
6. Add thermal-data quality status.
7. Return `Medium` when a signal is missing but not required.
8. Return `Low` when a signal is required for the proposed action and missing.

Tests:

- game GPU-bound candidate without GPU/frame data is low quality.
- CPU-affinity candidate can proceed with no GPU data.
- IRQ candidate requires strong IRQ data.
- VM knob candidate requires memory/PSI cliff evidence.

Done when:

- Data quality is not one-size-fits-all.

## 1.7 Improve observer/status output

Files:

- `stutter/src/autotune/status.rs`
- `stutter/src/autotune/human_output.rs`
- `stutter/src/autotune/decision_log.rs`
- `stutter/src/autotune/prometheus_metrics.rs`
- `stutter/src/autotune/tui_panel.rs`
- `stutter/src/commands/daemon.rs`
- `stutter/src/daemon/explain.rs`

Steps:

1. Add observer output fields:
   - focus group;
   - situation;
   - situation confidence;
   - top evidence;
   - blockers;
   - protected tasks count;
   - allowed mode;
   - candidate count;
   - top denied reason.
2. Add `stutter daemon status --explain-focus`.
3. Add `stutter daemon status --explain-situation`.
4. Add JSON output for both.
5. Update TUI compact lines.
6. Update Prometheus metrics:
   - `stutter_focus_confidence`;
   - `stutter_situation_kind`;
   - `stutter_candidates_available_total`;
   - `stutter_candidates_denied_total{reason=...}`.

Tests:

- JSON status includes focus/situation/evidence/blockers.
- text status does not leak titles unless explicitly enabled.
- prometheus output is stable and escaped.

Done when:

- In observe mode, users can understand exactly why no action was taken.

Implementation status:

- [x] 2026-05-15: Extended runtime JSON decision stream with situation confidence, top evidence, blockers, protected-task count, candidate count, and top denied/no-action reason from the planner.

## 1.8 Build observer replay fixtures

Files:

- new: `testdata/autotune/observer/*.json`
- `stutter/src/autotune/replay.rs`
- `stutter/src/autotune/history_replay.rs`
- `stutter/src/test_fixture_builder.rs`
- new tests under `stutter/src/autotune/situation.rs` or `runtime.rs`

Fixtures to add:

1. `idle_desktop.json`
2. `game_foreground_cpu_scheduler_pressure.json`
3. `game_foreground_gpu_bound.json`
4. `game_plus_recording_active.json`
5. `browser_foreground_cpu_pressure.json`
6. `browser_video_playback.json`
7. `compile_background_desktop_foreground.json`
8. `compile_foreground_cpu_bound.json`
9. `linker_io_pressure.json`
10. `vm_foreground_load.json`
11. `thermal_degraded_game.json`
12. `low_quality_drops.json`
13. `critical_realtime_warning.json`
14. `foreground_unavailable_hybrid_fallback.json`
15. `focus_flapping_prevented.json`

Done when:

- Observer behavior can be regression-tested without live eBPF.

---

# Milestone 2 — Safe local autotuner

Goal: turn the current CPU-affinity-only low-risk system into a general but still low-risk local autotune engine. This milestone should still avoid medium/high-risk mutation.

## 2.1 Replace profile-specific candidate model with a generic candidate model

Current problem:

- `CandidateAction` only represents `CpuAffinityProfile` and `Fake`.
- Many methods assume `profile_name()`, `tree_pid()`, and `cpu-affinity profile` semantics.

Files:

- `stutter/src/autotune/candidate.rs`
- `stutter/src/autotune/apply_low_risk.rs`
- `stutter/src/autotune/controller.rs`
- `stutter/src/autotune/runtime.rs`
- `stutter/src/autotune/candidate_memory.rs`
- `stutter/src/autotune/decision.rs`
- `stutter/src/autotune/human_output.rs`
- `stutter/src/autotune/report_overlay.rs`
- `stutter/src/autotune/prometheus_metrics.rs`

New shape:

```rust
pub enum CandidateAction {
    CpuAffinityProfile { profile_name: String, profile: Profile, tree_pid: u32 },
    Nice { action: NiceActionPlan },
    IoPrio { action: IoPrioActionPlan },
    Uclamp { action: UclampActionPlan },
    CgroupPlacement { action: CgroupPlacementActionPlan },
    IrqAffinity { action: IrqAffinityActionPlan },
    CpuPower { action: CpuPowerActionPlan },
    GpuPower { action: GpuPowerActionPlan },
    VmKnob { action: VmKnobActionPlan },
    Fake { action_id: ActionId, safety_class: SafetyClass },
}
```

Add methods:

```rust
impl CandidateAction {
    pub fn candidate_name(&self) -> &str;
    pub fn action_kind(&self) -> &'static str;
    pub fn action_id(&self) -> ActionId;
    pub fn descriptor(&self) -> ActionDescriptor;
    pub fn safety_class(&self) -> SafetyClass;
    pub fn effect_scope(&self) -> ActionEffectScope;
    pub fn target_root_pid(&self) -> Option<u32>;
    pub fn evidence(&self) -> &[CandidateEvidence];
    pub fn cooldown_key(&self) -> String;
    pub fn conflict_group(&self) -> ActionConflictGroup;
}
```

Steps:

1. Rename `profile_name()` to `candidate_name()` everywhere.
2. Keep `profile_name()` as deprecated helper only for CPU-affinity-specific tests, then remove.
3. Replace `tree_pid()` with `target_root_pid()`.
4. Move `suggestion_action_descriptor()` toward `candidate.descriptor()`.
5. Keep exhaustive matches so unsupported variants fail at compile time.
6. Update tests incrementally.

Done when:

- A new candidate family can be represented without pretending to be a CPU profile.

Implementation status:

- [x] 2026-05-15: Added generic `CandidateAction` variants and plan structs for nice, ionice, uclamp, cgroup, IRQ, CPU/GPU power, and VM knobs, plus `candidate_name`, `target_root_pid`, descriptors, evidence, cooldown keys, conflict groups, and objectives while keeping CPU-affinity compatibility helpers.

## 2.2 Add `CandidateProvider` trait and registry

Files:

- new: `stutter/src/autotune/providers/mod.rs`
- new: `stutter/src/autotune/providers/cpu_affinity.rs`
- new: `stutter/src/autotune/providers/nice.rs`
- new: `stutter/src/autotune/providers/ioprio.rs`
- new: `stutter/src/autotune/providers/uclamp.rs`
- later: providers for cgroup/IRQ/power/VM knobs
- `stutter/src/autotune/runtime.rs`
- `stutter/src/autotune/mod.rs`

Trait:

```rust
pub trait CandidateProvider {
    fn family(&self) -> &'static str;
    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal>;
}

pub struct CandidateProviderInput<'a> {
    pub observation: &'a AutotuneObservation,
    pub daemon_policy: &'a DaemonPolicy,
    pub capabilities: &'a DaemonCapabilities,
    pub system_health: &'a SystemHealthSnapshot,
    pub controller_state: &'a ControllerRuntimeState,
    pub profiles: &'a [Profile],
}
```

Steps:

1. Wrap existing `generate_profile_candidates()` inside `CpuAffinityProvider`.
2. Move `candidate_situation_rank()` into provider-specific ranking metadata.
3. Create a provider registry:
   - always registers CPU-affinity provider;
   - registers other providers in suggest-only mode after they are added;
   - filters by `enabled_action_families`/`denied_action_families`.
4. Let each provider return proposals with:
   - `candidate`;
   - evidence;
   - provider confidence;
   - deny reasons;
   - objective kind.
5. Add registry tests.

Done when:

- `AutotuneRuntime::select_candidate_for_observation()` asks providers instead of hardcoding profile generation.

Implementation status:

- [x] 2026-05-15: Added `CandidateProvider`, `CandidateProviderInput`, `CandidateProposal`, and a registry with CPU-affinity, nice, ionice, and uclamp providers; live runtime now plans through the registry for non-simulated candidates.

## 2.3 Add candidate evaluation and denial records

Current problem:

- `CandidateDryRunRecord` mostly describes dry-run output and eligibility.
- Denied candidates are not rich enough for “why did the watcher not tune?” UX.

Files:

- `stutter/src/autotune/candidate.rs`
- new: `stutter/src/autotune/planner.rs`
- `stutter/src/autotune/human_output.rs`
- `stutter/src/autotune/status.rs`

New types:

```rust
pub struct CandidateEvaluation {
    pub candidate_name: String,
    pub action_kind: String,
    pub descriptor: ActionDescriptor,
    pub provider: String,
    pub eligible: bool,
    pub deny_reasons: Vec<CandidateDenyReason>,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
    pub rank: Option<u32>,
    pub dry_run: Option<ActionState>,
}
```

Steps:

1. Keep `CandidateDryRunRecord` temporarily.
2. Add conversion from old dry-run records to `CandidateEvaluation`.
3. Add denial reasons:
   - disabled family;
   - denied family;
   - safety class too high;
   - effect scope too broad;
   - capability missing;
   - data quality low;
   - health degraded;
   - no explicit target;
   - focus low confidence;
   - critical realtime warning;
   - cooldown active;
   - conflict with active/kept action;
   - objective signal missing.
4. Expose top denied candidates in status.

Done when:

- The daemon can say “I saw a candidate, but it was blocked because X.”

Implementation status:

- [x] 2026-05-15: Added `CandidateEvaluation`, `CandidateDenyReason`, dry-run state capture, policy-context denial mapping, evidence, objectives, and provider/rank metadata in `autotune/planner.rs`.

## 2.4 Build a real `CandidatePlanner`

Files:

- new: `stutter/src/autotune/planner.rs`
- `stutter/src/autotune/runtime.rs`
- `stutter/src/autotune/controller.rs`
- `stutter/src/autotune/candidate_memory.rs`

Planner responsibilities:

1. Collect proposals from registered providers.
2. Attach policy context.
3. Run descriptor through `DaemonPolicy` for suggest/dry-run/apply intent.
4. Run dry-run only when allowed and cheap enough.
5. Filter or rank candidates.
6. Select one best candidate.
7. Return a full `PlanResult`, not just `Option<CandidateAction>`.

New output:

```rust
pub struct PlanResult {
    pub selected: Option<CandidateAction>,
    pub evaluations: Vec<CandidateEvaluation>,
    pub no_action_reason: Option<String>,
}
```

Steps:

1. Move `select_best_candidate_for_situation()` into `planner.rs` as legacy CPU-affinity ranking.
2. Add provider rank first, then policy eligibility, then cooldown, then confidence, then objective fit.
3. Add anti-flapping rules:
   - do not select candidate if focus changed recently;
   - do not select candidate if current situation changed recently;
   - do not select same conflict group during cooldown.
4. Add “investigate-only” outcomes for situations where evidence exists but action is too risky.
5. Store last `PlanResult` in runtime for status/TUI.

Done when:

- Runtime no longer contains planner-specific hardcoded candidate ranking.

Implementation status:

- [x] 2026-05-15: Added `CandidatePlanner` and `PlanResult`; runtime stores the last plan result and uses planner/provider ranking for live candidate selection, with legacy simulation ranking retained only for simulation candidates.

## 2.5 Keep low-risk apply CPU-affinity-only until planner is solid

Files:

- `stutter/src/autotune/apply_low_risk.rs`
- `stutter/src/autotune/runtime.rs`
- `stutter/src/autotune/shutdown.rs`
- `stutter/src/autotune/startup_recovery.rs`

Steps:

1. Keep low-risk apply only for `CpuAffinityProfile` at first.
2. Make unsupported variants produce structured rejection, not generic bail.
3. Rename `CpuAffinityCandidateExecutor` to something explicit like `CpuAffinityLowRiskExecutor`.
4. Add a trait object executor factory:
   - `executor_for_low_risk_candidate(candidate) -> Result<Box<dyn LowRiskActionExecutor>>`.
5. Add tests that non-affinity variants are suggested but not low-risk applied.
6. Add `apply-low-risk` status output that reports supported families.

Done when:

- General candidates can exist without accidentally being applied by low-risk mode.

Implementation status:

- [x] 2026-05-15: Renamed the low-risk executor to `CpuAffinityLowRiskExecutor`, added `executor_for_low_risk_candidate`, and made unsupported generic variants return structured low-risk rejection text instead of entering apply.

## 2.6 Make objective scoring workload-aware

Files:

- new: `stutter/src/autotune/objective.rs`
- `stutter/src/autotune/comparison.rs`
- `stutter/src/scorer.rs`
- `stutter/src/autotune/runtime.rs`
- `stutter/src/autotune/measurement.rs`

Objective examples:

```rust
pub enum ObjectiveKind {
    GameFramePacing,
    GameRunnableLatency,
    DesktopInteractivity,
    BrowserInteractivity,
    CompileThroughputWithForegroundProtection,
    IoLatency,
    IrqOverlapReduction,
    ThermalRecovery,
}
```

Steps:

1. Keep current `StutterScore` total as default objective.
2. Add objective-specific comparison functions.
3. For games:
   - prefer lower runnable latency on target game/render tasks;
   - consider frame p99/max regressions strongly;
   - avoid “improvement” if FPS/frame data got worse.
4. For browser/desktop:
   - prioritize foreground runnable latency and input responsiveness.
5. For compile:
   - goal is either compile throughput or foreground protection depending on focus.
6. For recording/media:
   - prioritize no frame/audio/recorder regressions.
7. Add objective to candidate metadata.
8. Store objective in history and decision logs.

Tests:

- candidate with lower total score but worse frame p99 is rejected for game frame objective.
- compile candidate that improves compile but hurts foreground browser is rejected when browser is focus.
- desktop objective rejects action that helps background workload but hurts compositor/input.

Done when:

- “Improved” means improved for the actual focused task, not just a global score drop.

Implementation status:

- [x] 2026-05-15: Added `ObjectiveKind` and `compare_for_objective`, wired live keep/revert comparison through candidate objectives, and added regression tests for game frame pacing, desktop interactivity, and default score behavior.

## 2.7 Strengthen baseline and measurement windows

Files:

- `stutter/src/autotune/baseline.rs`
- `stutter/src/autotune/measurement.rs`
- `stutter/src/autotune/context_segment.rs`
- `stutter/src/autotune/runtime.rs`

Steps:

1. Require stable workload identity across baseline and candidate windows.
2. Require stable focus group unless candidate explicitly targets focus switch behavior.
3. Require comparable context segment:
   - similar active target counts;
   - similar frame counts when frame objective;
   - similar GPU busy range when GPU objective;
   - similar PSI ranges when memory/IO objective;
   - similar route marker when available.
4. Add a `MeasurementGuard` wrapper that owns:
   - baseline status;
   - washout status;
   - candidate status;
   - comparison status.
5. Stop live experiment if comparison becomes invalid.

Tests:

- focus switch during candidate measurement reverts.
- target identity shift reverts.
- frame count mismatch marks low quality.
- context mismatch marks inconclusive/revert, not keep.

Done when:

- Live decisions are repeatable enough to trust.

## 2.8 Add apply-low-risk acceptance suite

Files:

- `stutter/src/autotune/simulation.rs`
- `stutter/src/daemon/acceptance.rs`
- `stutter/src/autotune/runtime.rs` tests
- new fixtures under `testdata/autotune/apply_low_risk/`

Scenarios:

1. observe never suggests/applies.
2. suggest emits candidate but does not apply.
3. apply-low-risk starts CPU-affinity experiment only with high confidence.
4. apply-low-risk rejects medium-risk nice/uclamp suggestions.
5. apply-low-risk reverts on regression.
6. apply-low-risk keeps on objective improvement.
7. apply-low-risk reverts on target exit.
8. apply-low-risk reverts on focus switch.
9. apply-low-risk reverts on drop counters.
10. apply-low-risk does not repeat same action during cooldown.
11. crash recovery rolls back applied candidate.
12. shutdown rollback works.

Done when:

- CPU-affinity local autotune is rock solid before adding more mutating families.

---

# Milestone 3 — Medium-risk process tuning

Goal: add reversible local/process/cgroup tuning families in suggest mode first, then explicitly unlock apply-medium-risk after the planner, policy, and rollback are stable.

## 3.1 Add `NiceProvider`

Existing action:

- `stutter/src/actions/nice.rs`
- Safety: `ReversibleMediumRisk`.

New provider:

- `stutter/src/autotune/providers/nice.rs`

Candidate strategy:

1. Never decrease priority of protected tasks.
2. For focused interactive workload:
   - suggest slightly higher priority for selected foreground process tree only if policy allows negative nice or safe range;
   - otherwise suggest lowering background helper/compile tasks.
3. For background compile while browser/game is foreground:
   - suggest increasing nice value for compiler workers.
4. For game focus:
   - avoid changing Wine/game main thread nice until enough evidence exists;
   - initially suggest for helpers/background only.

Implementation steps:

1. Add `CandidateAction::Nice` variant.
2. Add `NiceActionPlan` with target identities and requested nice value.
3. Convert plan to `NiceAction` from `actions/nice.rs`.
4. Add provider rules by situation.
5. Add dry-run and suggestion output.
6. Keep apply disabled until apply-medium-risk gate is implemented.

Tests:

- suggests lowering background compiler priority when browser focused.
- refuses protected audio/compositor/input tasks.
- refuses unknown root with low confidence.
- dry-run shows affected tasks.
- suggest output includes required mode `apply-medium-risk`.

Done when:

- Nice candidates are visible and auditable but not automatically applied in low-risk mode.

Implementation status:

- [x] 2026-05-15: Added `NiceProvider` and `CandidateAction::Nice` suggestion support with target identity, evidence, descriptor metadata, medium-risk safety, and protected-root exclusion.

## 3.2 Add `IoPrioProvider`

Existing action:

- `stutter/src/actions/ioprio.rs`
- Safety: `ReversibleMediumRisk`.

New provider:

- `stutter/src/autotune/providers/ioprio.rs`

Candidate strategy:

1. For foreground game/browser/media with background I/O pressure:
   - suggest lowering background I/O priority.
2. For focused compiler/linker causing user-visible stalls:
   - suggest best-effort or idle class for background non-focused I/O.
3. Do not boost random foreground I/O until evidence proves it helps.
4. Do not touch filesystem daemons, journal, pipewire, compositor, input stack.

Steps:

1. Add `CandidateAction::IoPrio`.
2. Add `IoPrioActionPlan` with class/level.
3. Add provider rules using `IoPressure`, `BrowserIoPressure`, `CompileLinkerPressure`.
4. Require block-I/O evidence or PSI I/O evidence before suggesting.
5. Add capability check using `DaemonCapabilities::ionice_available`.
6. Add dry-run to status.

Tests:

- no candidate without I/O pressure evidence.
- background writer gets idle/best-effort lowered candidate.
- foreground protected tasks skipped.
- missing ionice capability denies candidate.

Done when:

- I/O priority is controlled by workload evidence, not guesswork.

Implementation status:

- [x] 2026-05-15: Added `IoPrioProvider` and `CandidateAction::IoPrio` suggestion support gated on I/O-related situations and ionice capability, with idle I/O priority plans and objective metadata.

## 3.3 Add `UclampProvider`

Existing action:

- `stutter/src/actions/uclamp.rs`
- Safety: `ReversibleMediumRisk`.

New provider:

- `stutter/src/autotune/providers/uclamp.rs`

Candidate strategy:

1. For focused game/render or compositor pressure:
   - suggest modest `uclamp.min` for specific critical threads only.
2. For background compile while interactive app is focused:
   - suggest `uclamp.max` cap on compiler worker tree.
3. For thermal degraded state:
   - suggest lowering/clearing previously aggressive uclamp, not raising.
4. Never uclamp protected realtime tasks without an explicit protected-task policy.

Steps:

1. Add `CandidateAction::Uclamp`.
2. Add `UclampActionPlan` with target tasks and min/max values.
3. Require `DaemonCapabilities::uclamp_available`.
4. Add provider evidence:
   - scheduler pressure;
   - target class;
   - target thread names like render/compositor;
   - thermal state.
5. Start in suggest-only.
6. Add apply-medium-risk later.

Tests:

- render-thread candidate suggested only for game scheduler pressure.
- compile cap candidate suggested when compile is background and foreground interactive.
- thermal degraded blocks uclamp min increases.
- missing uclamp capability denies.

Done when:

- Uclamp is a precise per-task/cgroup tool, not a global boost hammer.

Implementation status:

- [x] 2026-05-15: Added `UclampProvider` and `CandidateAction::Uclamp` suggestion support gated on uclamp capability, health, and workload situation, with modest per-target min/max plans.

## 3.4 Add `CgroupProvider`

Existing action:

- `stutter/src/actions/cgroup.rs`
- Safety: `ReversibleMediumRisk`.

New provider:

- `stutter/src/autotune/providers/cgroup.rs`

Candidate strategy:

1. Use cgroup moves only when the target cgroups are explicitly configured.
2. Do not auto-create arbitrary system cgroups at first.
3. For workstation mode:
   - move background compile into a constrained cgroup;
   - keep foreground app/compositor/audio outside constrained group.
4. For game mode:
   - optionally move game tree into a preconfigured game cgroup with cpuset/cpu.weight already managed by user.

Steps:

1. Add config for named cgroup targets:
   - `interactive_cgroup`;
   - `background_cgroup`;
   - `game_cgroup`;
   - `compile_cgroup`.
2. Add `CandidateAction::CgroupPlacement`.
3. Add `CgroupPlacementActionPlan`.
4. Provider requires cgroup v2 capability.
5. Provider requires target cgroup allowlist.
6. Add policy denial if target cgroup not allowlisted.
7. Keep suggest-only until apply-medium-risk is unlocked.

Tests:

- no cgroup candidate without configured cgroup allowlist.
- candidate moves only target tree, not protected tasks.
- missing cgroup v2 denies.
- dry-run counts pending moves.

Done when:

- Cgroup tuning is explicit, reversible, and not silently system-wide.

Implementation status:

- [x] 2026-05-15: Added named cgroup target config/validation, policy allowlist propagation, `CgroupProvider`, active-task snapshots in observations, planner denial for non-allowlisted cgroup targets, and tests for no-allowlist, protected-task filtering, missing cgroup v2 capability denial, and invalid config paths.

## 3.5 Add protected-task and mutation exclusion layer

Files:

- new: `stutter/src/autotune/protection.rs`
- `stutter/src/focus/groups.rs`
- `stutter/src/focus/classify.rs`
- `stutter/src/autotune/observation.rs`
- all process-scoped providers

Protected categories:

1. compositor;
2. audio server/realtime audio;
3. input stack;
4. recorder/streamer when recording active;
5. display server/window manager;
6. kernel/system services;
7. root-owned service processes unless explicitly allowed;
8. unknown foreground-like processes with low confidence.

Steps:

1. Implement `ProtectionDecision`:
   - `Allowed`;
   - `Protected`;
   - `RequiresExplicitOptIn`;
   - `UnknownDeny`.
2. Add `mutation_allowed_for_task(task, candidate_family, observation)`.
3. Use it in all process-scoped providers.
4. Include protection deny reasons in candidate evaluations.
5. Add policy config for explicit opt-ins.

Tests:

- pipewire/wireplumber protected.
- sway/kwin/gnome-shell protected.
- OBS protected while recording.
- compiler workers not protected by default.
- game helper can be modified only when part of focused game tree.

Done when:

- Providers cannot accidentally target critical desktop/audio/input components.

Implementation status:

- [x] 2026-05-15: Added centralized `autotune/protection.rs` with `ProtectionDecision` and `mutation_allowed_for_pid`, plus provider integration for nice, ionice, and uclamp process-scoped suggestions.

## 3.6 Implement generic medium-risk executor path

Files:

- `stutter/src/autotune/apply_low_risk.rs` or new `stutter/src/autotune/apply.rs`
- `stutter/src/autotune/runtime.rs`
- `stutter/src/actions/runner.rs`
- `stutter/src/daemon/policy.rs`
- `stutter/src/daemon/config.rs`

Steps:

1. Rename `apply_low_risk.rs` to a more generic `apply.rs`, or add `apply_medium_risk.rs` and keep low-risk separate.
2. Add `CandidateActionExecutor` trait:
   - `candidate_name()`;
   - `descriptor()`;
   - `dry_run()`;
   - `apply()`;
   - `rollback()`.
3. Implement executor adapters for:
   - CPU affinity;
   - nice;
   - ionice;
   - uclamp;
   - cgroup.
4. Route every executor through `run_action_with_audit` / action runner.
5. Add `apply_medium_risk` mode path in runtime.
6. Require explicit config preset or CLI flag to enable medium-risk apply.
7. Keep high-risk blocked.

Tests:

- apply-low-risk still rejects medium-risk candidates.
- apply-medium-risk accepts reversible medium local/process/cgroup candidates only.
- missing rollback blocks apply.
- policy deny family blocks apply.
- capability missing blocks apply.
- dry-run succeeds without mutation.

Done when:

- Medium-risk local tuning is possible without weakening low-risk mode.

Implementation status:

- [x] 2026-05-15: Added generic `autotune/apply.rs` with `CandidateActionExecutor`, runner-backed executor adapters for CPU affinity, nice, ionice, uclamp, and cgroup candidates, medium-risk policy checks, rollback guard, and `run_apply_medium_risk_candidate`; updated daemon policy to allow explicit cgroup scope in apply-medium-risk; added a simulated runtime apply-medium-risk start path while keeping apply-low-risk CPU-affinity-only.

## 3.7 Add medium-risk rollback and crash recovery coverage

Files:

- `stutter/src/autotune/startup_recovery.rs`
- `stutter/src/autotune/emergency_restore.rs`
- `stutter/src/autotune/shutdown.rs`
- `stutter/src/actions/mod.rs`
- `stutter/src/profile_restore.rs`

Steps:

1. Ensure every new rollback token type is included in:
   - startup recovery;
   - emergency restore;
   - shutdown rollback;
   - daemon status.
2. Add “dirty journal” behavior for medium-risk actions.
3. Add one crash-recovery fixture per family.
4. Add rollback skip behavior for exited tasks.
5. Add rollback skip behavior for TID reuse/starttime mismatch.

Tests:

- startup recovery rolls back nice.
- startup recovery rolls back ionice.
- startup recovery rolls back uclamp.
- startup recovery rolls back cgroup.
- emergency restore aggregates partial failures.

Done when:

- Medium-risk actions are as recoverable as CPU-affinity actions.

Implementation status:

- [x] 2026-05-15: Verified medium-risk rollback tokens flow through startup recovery/manual emergency restore, switched cgroup rollback token safety classification from high-risk to reversible medium-risk to match `CgroupPlacementAction`, and added regression coverage for rollback-token safety mapping plus the existing all-token manual restore command test.

---

# Milestone 4 — System-adjacent tuning

Goal: add system-level action families as suggest/manual-first, then high-risk opt-in only. This is where the project becomes powerful and dangerous, so every step needs evidence and rollback.

## 4.1 Improve hardware and capability inventory

Files:

- `stutter/src/daemon/capabilities.rs`
- `stutter/src/daemon/health.rs`
- `stutter/src/hwmon.rs`
- `stutter/src/topology.rs`
- `stutter/src/irq_inspect.rs`
- new: `stutter/src/system_inventory.rs`

Steps:

1. Add `SystemInventory`:
   - CPU topology;
   - SMT sibling groups;
   - NUMA nodes if available;
   - cpufreq policy paths;
   - EPP/EPB support;
   - schedutil/performance governors available;
   - GPU DRM cards/render nodes;
   - hwmon mapping;
   - IRQ to device mapping;
   - current scx scheduler status if available;
   - memory/VM knob current values.
2. Store inventory hash in daemon state.
3. Block using learned profiles if inventory hash changes.
4. Surface inventory in `doctor` and `daemon doctor`.

Tests:

- fake sysfs inventory finds CPU policies.
- fake DRM inventory maps card to hwmon.
- topology hash changes when CPU layout changes.
- missing inventory blocks high-risk providers.

Done when:

- System-wide providers have a reliable map of what they are about to touch.

Implementation status:

- [x] 2026-05-15: Added `stutter/src/system_inventory.rs` with CPU cpufreq policy, DRM/render/hwmon, IRQ default affinity, sched_ext, VM knob snapshotting, stable inventory hashing, runtime observation signature wiring, and fake sysfs/DRM tests.

## 4.2 Add `IrqAffinityProvider`

Existing action:

- `stutter/src/actions/irq_affinity.rs`
- Safety: medium/high depending on risk.

Provider:

- `stutter/src/autotune/providers/irq_affinity.rs`

Candidate strategy:

1. Suggest only when IRQ pressure is classified.
2. Require strong evidence:
   - IRQ event overlap with scheduler/frame spikes;
   - stable IRQ device mapping;
   - known device class;
   - target CPU overlap problem.
3. Suggest moving IRQ away from game render/compositor critical CPUs or toward housekeeping CPUs.
4. For unknown IRQ/device mapping, recommend investigation only.
5. Initial apply should be manual/high-risk opt-in, not autonomous.

Steps:

1. Add `CandidateAction::IrqAffinity`.
2. Add provider using `irq_inspect` and recorded IRQ events.
3. Add `IrqAffinityEvidence` from existing action type or extend it.
4. Add deny reasons for missing evidence/mapping.
5. Add dry-run suggestions.
6. Add manual command output only when CLI policy would allow.

Tests:

- no IRQ candidate without IRQ pressure.
- no candidate with unstable mapping.
- known amdgpu/xhci IRQ can produce suggestion.
- high-risk candidate not applied without explicit high-risk opt-in.

Done when:

- IRQ tuning is evidence-driven and hard to accidentally apply.

Implementation status:

- [x] 2026-05-15: Added suggest-first `IrqAffinityProvider` that only proposes on `IrqPressure` with IRQ evidence and capability present; generated actions remain high-risk/policy-gated and non-autonomous by default.

## 4.3 Add `CpuPowerProvider`

Existing action:

- `stutter/src/actions/cpu_power.rs`
- Safety: high-risk.

Provider:

- `stutter/src/autotune/providers/cpu_power.rs`

Candidate strategy:

1. Suggest CPU power changes only when:
   - user explicitly enables the provider;
   - inventory knows cpufreq/EPP paths;
   - thermal/AC/battery health allows it;
   - current workload would plausibly benefit.
2. Game/interactive performance:
   - suggest performance governor/EPP performance for target policies only if safe.
3. Thermal degraded:
   - suggest reverting/relaxing performance settings.
4. Laptop safe preset should default to conservative or suggest-only.

Steps:

1. Add `CandidateAction::CpuPower`.
2. Add allowlist config for CPU policies.
3. Add provider rules for game CPU scheduler pressure and compile CPU bound.
4. Use `SystemHealthSnapshot` to block overheated/battery-limited cases.
5. Add high-risk warning in suggestion.
6. Keep autonomous apply disabled until explicit high-risk unlock.

Tests:

- default provider does not propose without enable flag.
- thermal degraded blocks performance increase.
- missing cpufreq path denies.
- allowlisted CPU policy generates dry-run.
- rollback token restores values in fake sysfs.

Done when:

- CPU power tuning is visible and reversible but never silent.

Implementation status:

- [x] 2026-05-15: Added suggest-first `CpuPowerProvider` using system inventory CPU policies and workload/health gates; high-risk CPU power candidates are routed through planner policy/dry-run denials by default.

## 4.4 Add `GpuPowerProvider`

Existing action:

- `stutter/src/actions/gpu_power.rs`
- Safety: high-risk.

Provider:

- `stutter/src/autotune/providers/gpu_power.rs`

Candidate strategy:

1. Suggest only for GPU-bound game/media/video situations.
2. Require GPU samples from the correct DRM card/render node.
3. Require explicit GPU card allowlist.
4. Block if temperature is high or power limit already causing degraded health.
5. Suggest revert/lower power when thermal degraded.

Steps:

1. Add `CandidateAction::GpuPower`.
2. Map hwmon/GPU samples to DRM card identity.
3. Add config for allowed cards and allowed knob ranges.
4. Add provider evidence:
   - GPU busy;
   - frame p99/max;
   - clocks/power if available;
   - thermal headroom.
5. Suggest high-risk manual command only when policy says manual high-risk is possible.

Tests:

- wrong GPU card blocks candidate.
- missing frame data blocks GPU-bound candidate.
- overheated GPU blocks performance candidate.
- fake sysfs rollback restores GPU values.

Done when:

- GPU tuning is tied to real GPU-bound evidence.

Implementation status:

- [x] 2026-05-15: Added suggest-first `GpuPowerProvider` gated on GPU-bound situations, health, and DRM inventory; generated GPU power candidates remain high-risk/manual by default.

## 4.5 Add `VmKnobProvider`

Existing action:

- `stutter/src/actions/vm_knobs.rs`
- Safety: high-risk.

Provider:

- `stutter/src/autotune/providers/vm_knobs.rs`

Candidate strategy:

1. Suggest VM knob changes only for memory/IO latency cliff situations.
2. Require PSI evidence and/or major fault evidence.
3. Require explicit allowlist of exact paths and values.
4. Never apply by default.
5. Prefer suggestions like “test swappiness X” rather than live auto mutation.

Steps:

1. Add `CandidateAction::VmKnob`.
2. Add provider rules for memory pressure/fault storms.
3. Add exact allowlist config.
4. Add objective `ThermalRecovery`/`IoLatency`/`DesktopInteractivity` depending on knob.
5. Keep suggest/manual-only.

Tests:

- no candidate without PSI/fault evidence.
- path not allowlisted denied.
- high-risk not applied without explicit unlock.
- fake proc/sys rollback restores knob.

Done when:

- VM knobs are treated as dangerous experiments, not routine tuning.

Implementation status:

- [x] 2026-05-15: Added suggest-first `VmKnobProvider` for I/O pressure situations with exact reversible knob plan metadata; default policy still denies autonomous high-risk mutation.

## 4.6 Add scheduler/scx provider

Current source has `stutter/src/scx.rs`, but no full action provider for switching/tuning sched_ext policies.

New files:

- `stutter/src/actions/scx.rs`
- `stutter/src/autotune/providers/scx.rs`
- update `stutter/src/actions/mod.rs`

Candidate strategy:

1. Initial version is suggest-only.
2. Detect current scx scheduler and config if possible.
3. Suggest scheduler profile changes only when:
   - sched_ext capability exists;
   - user configured allowed scheduler commands;
   - the current workload matches a known policy.
4. For your own performance direction, this could eventually support scx_lavd gaming presets, but it must be explicit and rollbackable.

Action design:

1. `ScxSchedulerAction` should not execute arbitrary shell by default.
2. Use an allowlisted command template or service control operation.
3. Rollback should restore previous scheduler/service/profile.
4. If rollback cannot be guaranteed, safety class is `HighRisk` with `RollbackRequirement::BestEffortOnly` or `Unavailable`, meaning no autonomous apply.

Tests:

- no candidate without sched_ext capability.
- arbitrary command rejected.
- allowlisted command dry-run emits expected plan.
- unavailable rollback blocks autonomous apply.

Done when:

- Scheduler tuning is possible without creating a root shell disguised as a tuning provider.

## 4.7 Implement high-risk manual/apply gate

Files:

- `stutter/src/daemon/config.rs`
- `stutter/src/daemon/policy.rs`
- `stutter/src/commands/daemon.rs`
- `stutter/src/commands/autotune.rs`
- `docs/DAEMON_CONTRACT.md`
- `docs/AUTOTUNE_SAFETY.md`

Steps:

1. Add config fields requiring explicit high-risk unlock:
   - `allow_high_risk = true`;
   - `allow_system_wide_actions = true`;
   - exact allowed families;
   - exact allowed devices/paths.
2. Require a scary CLI flag for high-risk live apply, not just config.
3. Add policy explanation lines that show every high-risk gate.
4. Refuse remote high-risk unless a separate remote policy explicitly allows it; default remains no.
5. Add `--dry-run` and `--manual-command-only` flows.

Tests:

- high-risk action rejected by default.
- high-risk rejected when family not allowlisted.
- high-risk rejected when system-wide not allowed.
- high-risk accepted only when all explicit gates are present.
- remote high-risk rejected by default.

Done when:

- High-risk tuning is impossible to enable accidentally.

## 4.8 Add cross-action conflict model

Files:

- new: `stutter/src/autotune/conflicts.rs`
- `stutter/src/autotune/planner.rs`
- `stutter/src/autotune/kept.rs`
- `stutter/src/autotune/state.rs`

Conflict groups:

- CPU affinity / cpuset / cgroup cpuset conflict.
- Nice/uclamp/cgroup cpu.weight conflict.
- IRQ affinity conflicts with CPU isolation plans.
- CPU power conflicts with thermal recovery plans.
- GPU power conflicts with thermal recovery plans.
- VM knobs conflict with memory-pressure recovery plans.

Steps:

1. Add `ActionConflictGroup` enum.
2. Each candidate declares one or more conflict groups.
3. Planner rejects candidates that conflict with active experiment.
4. Planner may reject candidates that conflict with kept profile unless the new candidate is a rollback/replacement plan.
5. Add status output for conflict denial.

Tests:

- active CPU affinity experiment blocks cgroup cpuset candidate.
- thermal recovery blocks CPU/GPU performance boost.
- independent nice candidate can coexist only after previous experiment is kept and conflict-free.

Done when:

- The daemon does not stack contradictory optimizations.

Implementation status:

- [x] 2026-05-15: Extended `ActionConflictGroup` with symmetric conflict rules, added `CandidateAction::conflicts_with`, and made the planner deny candidates that conflict with active experiments or kept actions, with conflict denial messages/tests covering CPU placement vs cgroup placement, thermal recovery vs power groups, and independent nice vs kept CPU placement.

---

# Milestone 5 — Full watcher

Goal: convert the mature observer/planner/action system into an always-on service that adapts across workloads and remembers what worked.

## 5.1 Make daemon runtime the main product path

Files:

- `stutter/src/daemon/runtime.rs`
- `stutter/src/daemon/autotune.rs`
- `stutter/src/daemon/monitor.rs`
- `stutter/src/service.rs`
- `stutter/src/commands/service.rs`
- `contrib/openrc/stutter`
- packaging files

Steps:

1. Ensure service startup runs startup recovery before monitoring.
2. Start monitor subsystem.
3. Start autotune subsystem in configured mode.
4. Persist daemon state snapshots regularly.
5. On stop, perform configured rollback-on-exit.
6. On crash/restart, inspect journal and restore before planning.
7. Make OpenRC service path first-class since this project targets Gentoo/OpenRC well.
8. Keep systemd packaging optional.

Tests:

- fake daemon runtime starts in observe mode.
- startup recovery runs before candidate planning.
- stop event triggers rollback.
- crash journal rolls back before new apply.

Done when:

- The daemon path, not one-shot autotune, is the main integration target.

## 5.2 Add privilege separation as a real boundary

Files:

- `stutter/src/daemon/privilege.rs`
- `stutter/src/agent.rs`
- `stutter/src/remote.rs`
- `stutter/src/commands/agent.rs`
- `stutter/src/actions/runner.rs`

Steps:

1. Keep read-only observer available to normal user when possible.
2. Route privileged mutations through a privileged worker/agent.
3. Keep operation allowlist extremely small.
4. Never let remote TCP request privileged operations directly without local policy and auth.
5. Audit every privileged request.
6. Add per-operation policy check inside privileged worker too, not only client side.

Tests:

- UI client cannot execute privileged mutation directly.
- loopback remote still requires auth and policy.
- remote non-loopback denied by default.
- privileged worker rejects non-allowlisted operation.

Done when:

- Running the watcher does not require turning the whole UI/client into a permanent root process.

## 5.3 Add workload profile memory

Existing source:

- `stutter/src/daemon/state.rs` already has profile memory concepts.
- `stutter/src/autotune/candidate_memory.rs` records candidate outcomes.

Steps:

1. Define stable workload key:
   - executable identity;
   - focus kind;
   - class distribution;
   - topology hash;
   - kernel/scx state;
   - GPU identity if relevant.
2. Store candidate outcomes per workload key.
3. Store environment hash with every kept candidate.
4. Invalidate memory when:
   - kernel changes;
   - CPU topology changes;
   - GPU changes;
   - scx scheduler changes;
   - config policy changes;
   - action implementation version changes.
5. Use memory to avoid retrying failed candidates.
6. Use memory to prefer historically successful candidates.
7. Add a `stutter daemon memory` command:
   - list;
   - explain;
   - forget workload;
   - forget candidate;
   - export JSON.

Tests:

- failed candidate is cooled down/avoided.
- kept candidate preferred for same workload.
- topology hash mismatch invalidates memory.
- forget command removes target entries only.

Done when:

- The watcher learns conservatively without becoming sticky across incompatible environments.

Implementation status:

- [x] 2026-05-15: Candidate memory context now consumes observation workload identity, executable dev/ino, class distribution, and inventory/topology signature so learned outcomes are partitioned by workload/environment compatibility.

## 5.4 Add full workload policy matrix

Files:

- new: `stutter/src/autotune/workload_policy.rs`
- `stutter/src/autotune/planner.rs`
- `stutter/src/daemon/config.rs`
- docs

Matrix examples:

### Game focused

Allowed suggestions:

- CPU affinity;
- uclamp targeted render/compositor;
- GPU power suggestion;
- IRQ suggestion;
- CPU power suggestion;
- background nice/ionice reduction.

Autonomous default:

- low-risk CPU affinity only.

### Browser focused

Allowed suggestions:

- background nice/ionice/uclamp max;
- protect browser foreground;
- maybe CPU affinity for noisy background only.

Autonomous default:

- none or medium-risk only with explicit workstation preset.

### Compile focused

Allowed suggestions:

- compile throughput profile;
- cgroup placement;
- uclamp max/min depending on intent.

Autonomous default:

- medium-risk workstation preset only.

### Recording active

Allowed suggestions:

- protect recorder/encoder/audio/compositor;
- prevent game-only aggressive isolation that hurts recording.

Autonomous default:

- extremely conservative.

### Media playback

Allowed suggestions:

- protect media/audio/video path;
- avoid background I/O/CPU contention.

### VM load

Allowed suggestions:

- cgroup/uclamp/CPU affinity if explicit VM policy configured.

### Idle

Allowed suggestions:

- revert kept performance actions if configured;
- power-saving suggestions.

Steps:

1. Implement policy matrix as data, not hardcoded scattered matches.
2. Each situation maps to allowed provider families and objective kinds.
3. Matrix can be overridden in config.
4. Matrix output appears in `daemon policy explain`.

Tests:

- game situation enables game providers.
- recording active blocks game-only aggressive candidates.
- idle situation produces revert/power-save suggestions only.
- browser foreground blocks compile-throughput optimization.

Done when:

- The watcher behaves differently for different tasks instead of treating everything like a game.

Implementation status:

- [x] 2026-05-15: Added `autotune/workload_policy.rs` as a data matrix mapping situations to allowed action families, objectives, and autonomous families; integrated it into planner candidate evaluation; and added tests for game provider enablement, recording blocking game-only actions, idle no-op behavior through the existing planner gate, and browser foreground blocking compile-throughput cgroup optimization.

## 5.5 Add steady-state kept-action management

Current controller keeps a low-risk profile with rollback available. Full watcher needs to manage multiple possible kept effects without stacking chaos.

Files:

- `stutter/src/autotune/kept.rs`
- `stutter/src/autotune/conflicts.rs`
- `stutter/src/autotune/runtime.rs`
- `stutter/src/daemon/state.rs`

Steps:

1. Represent kept actions as a set, not a single active profile.
2. Each kept action has:
   - action id;
   - conflict groups;
   - workload key;
   - environment hash;
   - rollback token;
   - applied time;
   - last verified time;
   - objective result.
3. Only one new experiment at a time, but multiple kept actions may exist if conflict-free.
4. Add periodic revalidation of kept actions.
5. Revert kept actions when:
   - focus changes to incompatible workload;
   - environment changes;
   - health degrades;
   - config denies it;
   - user disables daemon;
   - objective no longer holds.

Tests:

- conflict-free kept actions can coexist.
- conflicting new action requires replacing/reverting old action.
- focus switch triggers incompatible kept action rollback.
- health degraded reverts power/performance actions.

Done when:

- The daemon can live beyond one experiment without accumulating stale state.

## 5.6 Add self-healing execution

Existing source:

- `stutter/src/daemon/watchdog.rs` recommends actions.
- Runtime needs to execute appropriate safe self-healing actions.

Steps:

1. Convert watchdog recommended actions into daemon transitions:
   - restart monitor subsystem;
   - clear stale target;
   - pause autotune;
   - rollback active experiment;
   - enter observe-only;
   - fault hard.
2. Make rollback action highest priority.
3. Add rate-limiting for subsystem restarts.
4. Persist self-healing actions in daemon state/audit.
5. Expose in `daemon status --explain-last`.

Tests:

- drop counters high pauses autotune.
- dirty rollback journal triggers rollback/fault.
- stuck action phase faults or rolls back.
- monitor event rate zero restarts or pauses.

Done when:

- The daemon responds to its own failure modes instead of only reporting them.

## 5.7 Add user-facing control plane

Files:

- `stutter/src/commands/daemon.rs`
- `stutter/src/commands/autotune.rs`
- `stutter/src/tui.rs`
- `stutter/src/autotune/tui_panel.rs`
- docs

Commands:

1. `stutter daemon status --json`
2. `stutter daemon status --explain-focus`
3. `stutter daemon status --explain-situation`
4. `stutter daemon status --explain-candidates`
5. `stutter daemon watch --verbose`
6. `stutter daemon pause-autotune`
7. `stutter daemon resume-autotune`
8. `stutter daemon emergency-restore`
9. `stutter daemon memory list`
10. `stutter daemon memory forget`
11. `stutter daemon policy explain`
12. `stutter daemon dry-run-now`

Output rules:

- Always show mode.
- Always show whether mutation is possible.
- Always show active action and rollback status.
- Always show last decision and reason.
- Never leak window titles unless explicitly enabled.
- Never hide health/data-quality blockers.

Tests:

- JSON schema stable.
- text includes no unsafe title by default.
- pause/resume state persists.
- emergency restore clears dirty journal.

Done when:

- A user can trust and control the daemon without reading logs.

## 5.8 Add full-system simulation suite

Files:

- `stutter/src/autotune/simulation.rs`
- `stutter/src/daemon/acceptance.rs`
- `stutter/src/daemon/soak.rs`
- new: `testdata/autotune/full_system/*.json`

Scenarios:

1. boot into observe mode.
2. start game, detect game focus.
3. suggest game CPU-affinity candidate.
4. apply low-risk candidate.
5. game improves, keep.
6. OBS starts recording, game-only action becomes incompatible or protected.
7. browser becomes foreground, game action reverts or stays only if harmless.
8. compile starts in background, suggest background nice/ionice/uclamp cap.
9. thermal degrades, power candidates blocked and performance actions revert.
10. suspend/resume happens, daemon pauses, revalidates, resumes observe.
11. target exits, rollback happens.
12. daemon restarts after crash, startup recovery restores.
13. high-risk provider suggests but does not apply by default.
14. config enables high-risk dry-run, still no apply without explicit unlock.
15. dirty journal causes emergency restore path.

Done when:

- The final product behavior is tested as a story, not just as isolated functions.

## 5.9 Add long soak and overhead budgets

Files:

- `stutter/src/daemon/soak.rs`
- `stutter/src/daemon/overhead.rs`
- `stutter/src/session/sinks.rs`
- `stutter/src/recorder/retention.rs`
- CI workflow if feasible

Steps:

1. Define overhead budgets:
   - daemon CPU usage;
   - memory growth;
   - file descriptor count;
   - disk growth;
   - event channel drops;
   - decision log growth.
2. Add fake soak tests for many hours of simulated events.
3. Add retention tests for decision/history logs.
4. Add bounded channels and explicit drop counters for non-critical outputs.
5. Add status warning when overhead budget exceeded.

Tests:

- observe-only soak passes.
- apply-low-risk soak passes.
- medium-risk suggest soak passes.
- decision logs rotate/retain correctly.
- no unbounded memory growth in fake scenario.

Done when:

- The watcher can be left on permanently.

## 5.10 Final release hardening

Files:

- `README.md`
- `docs/INSTALL.md`
- `docs/SAFETY.md`
- `docs/AUTOTUNE_SAFETY.md`
- `docs/DAEMON_CONTRACT.md`
- `docs/PACKAGING.md`
- `contrib/openrc/stutter`
- `.github/workflows/ci.yml`

Steps:

1. Document exact supported modes.
2. Document default-denied high-risk actions.
3. Document emergency restore.
4. Document recommended Gentoo/OpenRC install.
5. Add packaging notes for capabilities/root helper.
6. Add known limitations:
   - compositor support;
   - foreground provider support;
   - kernel feature requirements;
   - hardware device support.
7. Add “first run” workflow:
   - `doctor`;
   - observe;
   - suggest;
   - dry-run;
   - apply-low-risk;
   - emergency restore.
8. Add examples for:
   - gaming desktop;
   - workstation compile protection;
   - laptop safe mode;
   - suggest-only high-risk GPU/CPU power.

Done when:

- Users can install, understand, safely disable, and recover the daemon.

---

# Recommended implementation order

Do not implement by action family first. Implement by safety maturity.

1. `SituationKind` unification and history fix.
2. `SituationClassification` module.
3. Richer `AutotuneObservation`.
4. Observer replay fixtures.
5. Generic `CandidateAction` naming cleanup.
6. `CandidateProvider` trait and CPU-affinity provider wrapper.
7. `CandidatePlanner` and denial records.
8. Objective scoring.
9. Apply-low-risk acceptance suite.
10. `NiceProvider` suggest-only.
11. `IoPrioProvider` suggest-only.
12. `UclampProvider` suggest-only.
13. Protected-task mutation exclusion layer.
14. Generic medium-risk executor.
15. Apply-medium-risk opt-in.
16. Cgroup provider.
17. Inventory system.
18. IRQ provider suggest/manual.
19. CPU power provider suggest/manual.
20. GPU power provider suggest/manual.
21. VM knob provider suggest/manual.
22. SCX provider suggest/manual.
23. Conflict model.
24. Workload policy matrix.
25. Profile memory.
26. Kept-action set management.
27. Daemon runtime/service path.
28. Privilege separation hardening.
29. Self-healing execution.
30. Full-system simulation suite.
31. Soak/overhead budgets.
32. Release docs and packaging.

---

# PR slicing recommendation

Smallest practical PRs:

1. `situation-kind-unification`: no behavior change except correct history labels.
2. `situation-classifier-v1`: pure classifier + tests.
3. `observation-v2`: richer observation, still no new apply.
4. `observer-fixtures`: replay data and regression tests.
5. `candidate-name-cleanup`: `profile_name()` -> `candidate_name()`.
6. `candidate-provider-trait`: introduce trait and CPU-affinity wrapper.
7. `planner-v1`: `PlanResult`, evaluations, denial reasons.
8. `status-candidate-explain`: expose planner output.
9. `objective-v1`: objective-aware comparison.
10. `low-risk-acceptance`: end-to-end CPU-affinity simulation tests.
11. `nice-provider-suggest`: suggestions only.
12. `ioprio-provider-suggest`: suggestions only.
13. `uclamp-provider-suggest`: suggestions only.
14. `protected-tasks`: centralized mutation exclusion.
15. `generic-executor`: medium executor infrastructure, still disabled.
16. `apply-medium-risk-gate`: explicit config/CLI unlock.
17. `cgroup-provider-suggest-apply-medium`: cgroup provider after medium gate.
18. `system-inventory`: CPU/GPU/IRQ/sysfs inventory.
19. `irq-provider-suggest`: no autonomous apply.
20. `cpu-power-provider-suggest`: no autonomous apply.
21. `gpu-power-provider-suggest`: no autonomous apply.
22. `vm-knob-provider-suggest`: no autonomous apply.
23. `scx-provider-suggest`: no autonomous apply.
24. `conflict-model`: action conflict groups.
25. `workload-policy-matrix`: data-driven provider allowlist per situation.
26. `profile-memory`: workload/action outcome memory.
27. `kept-action-set`: multiple kept actions with conflicts.
28. `daemon-service-integration`: daemon as main path.
29. `privileged-worker-enforcement`: policy in privileged worker.
30. `watchdog-self-healing`: execute safe recovery actions.
31. `full-system-simulation`: final boss test suite.
32. `soak-overhead-retention`: permanent-service hardening.
33. `docs-release`: docs, OpenRC, safety, packaging.

---

# Non-negotiable safety rules while implementing

1. No new provider should mutate directly.
2. No new provider should bypass `DaemonPolicy`.
3. No action without rollback should be autonomously applied.
4. High-risk actions must remain suggest/manual-only until explicit high-risk policy, CLI, docs, and tests exist.
5. Do not stack experiments.
6. Do not trust foreground title by default.
7. Do not target protected audio/input/compositor/recorder tasks automatically.
8. Do not use global score as the only keep/revert metric for every workload.
9. Do not add new probes unless a candidate/action or diagnosis actually uses them.
10. Do not let config/docs claim support before runtime actually enforces it.
