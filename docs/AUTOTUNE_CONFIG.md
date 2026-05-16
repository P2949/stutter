# Auto-tune Configuration

`stutter` supports daemon autotune configuration through the user config parser in `stutter/src/config_file.rs`.

This document describes the active `[autotune]` configuration surface plus the policy fields used by the daemon runtime.

## Example

```toml
[autotune]
mode = "observe"             # observe | suggest | apply-low-risk | apply-medium-risk
target = "watch-process"
watch_process = "Game.exe"
preset = "diagnosis"

max_safety_class = "ReversibleLowRisk"
one_action_at_a_time = true
rollback_on_exit = true
rollback_on_crash_recovery = true

baseline_window_seconds = 30
candidate_window_seconds = 30
washout_seconds = 10
cooldown_seconds = 60

min_scored_samples = 100
min_scored_intervals = 5
max_drop_counters = 0
require_stable_target_identity = true

min_improvement_percent = 12.5
max_regression_percent = 7.5
max_frame_p99_regression_ms = 2.0
max_runnable_p99_regression_ms = 1.0

allowed_actions = [
  "cpu_affinity_profile"
]

denied_actions = [
  "irq_affinity",
  "gpu_power_profile",
  "global_cpu_governor"
]
```

## Mode

`mode` controls how far the controller may go.

```text
observe:
  Collect evidence only.
  This is the default.

suggest:
  Collect evidence and emit candidate actions.
  Do not apply changes.

apply-low-risk:
  May apply actions up to ReversibleLowRisk.
  Must still satisfy all safety gates, data-quality gates, verification, audit logging, and rollback requirements.

apply-medium-risk:
  May apply actions up to ReversibleMediumRisk.
  Must require explicit configuration and stronger preflight than apply-low-risk.
```

`HighRisk` actions are not enabled by any mode in this contract. They require manual approval or a separate explicit future configuration key.

## Target

`target` selects the future target-resolution strategy.

```text
watch-process:
  Resolve the target from watch_process.

tree-pid:
  Resolve the target from an explicit process-tree root.

cgroupv2:
  Resolve the target from an explicit cgroup v2 path.
```

`watch_process` names the process to wait for when `target = "watch-process"`.

`preset` names the monitoring preset used for evidence collection. The preset must not silently enable high-risk actions.

## Safety

`max_safety_class` is the highest action safety class the controller may select. Valid values are:

```text
ObserveOnly
ReversibleLowRisk
ReversibleMediumRisk
HighRisk
```

`one_action_at_a_time = true` means the controller must not overlap experiments. This should be true for autonomous mode.

`rollback_on_exit = true` means a live autonomous session must rollback applied changes when it exits unless the user explicitly keeps them.

`rollback_on_crash_recovery = true` means the next run must attempt rollback from durable restore state before planning new actions.

## Timing Windows

`baseline_window_seconds` controls how long to measure before planning a candidate.

`candidate_window_seconds` controls how long to measure after applying a candidate.

`washout_seconds` controls how long to wait after applying or reverting before scoring measurements.

`cooldown_seconds` controls how long to wait after keeping or reverting before planning another candidate.

## Data-quality Gates

`min_scored_samples` is the minimum number of scored latency samples required before candidate planning or final scoring.

`min_scored_intervals` is the minimum number of scored intervals required before candidate planning or final scoring.

`max_drop_counters` is the maximum allowed eBPF drop-counter value for autonomous action. A value of `0` means any observed drop blocks action.

`require_stable_target_identity = true` means target process identity must remain stable across baseline and candidate windows.

Data-quality failure blocks action before apply and forces revert during measurement.

## Scoring Gates

`min_improvement_percent` is the minimum improvement required to keep an applied candidate.

`max_regression_percent` is the maximum tolerated regression before a candidate must be reverted.

`max_frame_p99_regression_ms` is the maximum tolerated frame p99 regression in milliseconds.

`max_runnable_p99_regression_ms` is the maximum tolerated runnable-latency p99 regression in milliseconds.

If scoring is inconclusive, the controller must revert unless an explicit future policy says to keep the baseline.

## Action Lists

`daemon_enabled_action_families` is an allowlist of action families the daemon may consider.

`daemon_denied_action_families` is a denylist of action families the daemon must never apply.

The denylist wins over the allowlist.

Current action family names are:

```text
cpu_affinity_profile
nice
ionice
uclamp
cgroup_placement
irq_affinity
cpu_power
gpu_power
vm_knob
```

## Workload Policy Rules

`[autotune.workload_policy]` can override the built-in workload policy matrix.

Empty workload policy config means built-in defaults are used.

Each rule overrides one situation. Any situation not listed keeps its built-in default.

```toml
[autotune]

[[autotune.workload_policy.rules]]
situation = "browser_focused"
allowed_families = ["nice", "ionice", "uclamp"]
allowed_objectives = ["browser_interactivity", "desktop_interactivity"]
autonomous_families = []
```

`allowed_families` controls which action families may be proposed for that situation.

`allowed_objectives` controls which planner objectives are allowed. An empty list means all objectives are allowed for that rule.

`autonomous_families` controls which allowed families may be selected in autonomous apply modes. An empty list is valid and means the rule allows suggestions but no autonomous apply for that situation.

Invalid action family names, invalid objective names, duplicate situation rules, and conflicting workload policy locations produce config diagnostics and validation errors.

The legacy alias `[[autotune.workload_policy_rules]]` is still accepted, but new config should use `[[autotune.workload_policy.rules]]`.
