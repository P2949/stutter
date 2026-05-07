# Auto-tune Safety

`stutter` uses safety classes to keep observation, recommendation, and system-changing actions separate. Future autonomous tuning must treat these classes as a policy boundary, not just as labels for audit output.

The Rust source already defines these categories in `stutter/src/actions/mod.rs` as `SafetyClass::ObserveOnly`, `SafetyClass::ReversibleLowRisk`, `SafetyClass::ReversibleMediumRisk`, and `SafetyClass::HighRisk`.

## Risk Classes

```text
ObserveOnly:
  Reads data only.
  Example: report, advisor, dry-run candidate generation.

ReversibleLowRisk:
  Per-process or per-thread change.
  Easy rollback.
  Should not affect unrelated system tasks.
  Example: CPU affinity for target process tree.

ReversibleMediumRisk:
  System or cgroup changes with bounded blast radius.
  Needs stronger preflight and cooldown.
  Example: cgroup placement, uclamp, nice, ioprio.

HighRisk:
  Broad system-wide or hardware-affecting changes.
  Manual approval or explicit config only.
  Example: CPU governor/EPP globally, IRQ affinity, GPU power profile, THP/compaction knobs.
```

## Autonomous Mode Expectations

`ObserveOnly` actions may run by default because they only read existing data or generate dry-run candidates.

`ReversibleLowRisk` actions may be eligible for autonomous mode only when they have a working preflight check, dry-run behavior, apply step, verify step, rollback path, and durable audit event.

`ReversibleMediumRisk` actions require stronger evidence, stronger preflight checks, bounded scope, explicit cooldown, and a rollback path before they are eligible for autonomous mode.

`HighRisk` actions must not be enabled by default. They require manual approval or explicit configuration and must never be silently selected by an autonomous controller.

Any action that cannot be rolled back is ineligible for autonomous mode regardless of its nominal safety class.
