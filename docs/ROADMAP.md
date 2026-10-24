# Roadmap

`stutter` is moving from profiler toward trustworthy recommender in small, auditable steps.

## Project status

Current engineering phase: **Post-v0.1 Evidence Hardening**. Core Validation is
complete and `v0.1.0` is released ([releases/v0.1.0.md](releases/v0.1.0.md)); the staged
plan is in [CORE_VALIDATION_PLAN.md](CORE_VALIDATION_PLAN.md) and the historical ledger in
[CORE_VALIDATION_STATUS.md](CORE_VALIDATION_STATUS.md).

The v0.1 evidence left honest limitations: diagnosis recall was 0.0 at the current
thresholds, StutterScore was non-monotonic in both studies, and the multi-kernel cycle needed
manual reboot orchestration. This phase fixes the ground truth before any tuning: a stronger
diagnosis corpus with matched controls and independent target-impact proof, recalibration only
against it (with a held-out set), a broader StutterScore workload matrix without changing
weights, release-publication and lifecycle hardening, and persistent autonomous multi-kernel
resume on NixOS. No new user-facing Supported or mutating functionality is added until this
work is complete: new mutation providers, system-wide tunables, high-risk autotune, scheduler
integrations, major remote-control features, new UI surfaces and unrelated crate refactors
remain deferred.

`stutter` is an experimental open-source project. Near-term work prioritizes
trustworthy measurement, repeatability, explainability, conservative
recommendation, and rollback over expanding the number of system settings it
can modify.

Experimental or internal subsystems may exist in-tree before they are considered
safe or supported defaults. Roadmap status describes user-facing maturity, not
whether prototype code exists.

## Stages

| Stage | Status | Notes |
| --- | --- | --- |
| Profiler | Completed | Records runnable latency, frame correlation, optional IRQ/GPU/block-I/O/fault context, and data-quality notes. |
| Manual diagnosis | Completed | Reports cautious diagnosis candidates and supporting evidence. |
| Profile benchmarker | Current | `tune` benchmarks explicit CPU-affinity profile sets and records per-candidate artifacts. |
| Profile recommender | Current | `tune` writes JSON, Markdown, and HTML recommendation artifacts. `recommend --html` shows A/B distributions, bootstrap CI bands, effect size, sample counts, noise ratios, and underpowered warnings. |
| Advisor daemon | Current | `advisor --watch-runs` watches completed runs and emits conservative offline recommendations with structured fix plans. |
| Fix validation | Current | `recommend --fix-plan` validates advisor hypotheses against repeated baseline/tune A/B evidence and reports validated/rejected/underpowered/inconclusive/invalid-experiment status. |
| Limited auto-tuner | In development / gated | Runtime, planning, live experiment, rollback, audit, and emergency-restore infrastructure exist. User-facing enablement remains gated on safety validation, policy coverage, and end-to-end recovery confidence. |
| Broader optimizer | Experimental / internal | Providers and candidate paths exist for CPU affinity, nice, ioprio, uclamp, cgroup placement, IRQ affinity, CPU power, VM knobs, and GPU power. Broader user-facing enablement remains future work pending stronger evidence, policy hardening, and clearer operator controls. |

### Autotune Implementation Status

The autotune subsystem is partially implemented in-tree. Roadmap status refers to
safe user-facing enablement, not absence of code. Experimental/internal paths may
exist before they are considered supported defaults.

### Objective comparison maturity

Per-objective maturity, primary metrics, fallbacks and guardrails are defined by
`ObjectiveKind::descriptor()` (`stutter/src/autotune/objective/descriptor.rs`) and
generated into [AUTOTUNE_OBJECTIVES.md](AUTOTUNE_OBJECTIVES.md). This roadmap
intentionally does not duplicate that table.

Capability tiers (Supported/Experimental/Internal/Disabled) and graduation requirements are
defined in [CAPABILITIES.md](CAPABILITIES.md) and printed by `stutter capabilities`.

## Direction

Near-term work should improve trust, repeatability, and rollback before adding broader system changes:

- Better comparability checks for tune runs.
- More guided proof workflows around `advisor --json`, `tune`, and `recommend --fix-plan`.
- More example profile sets and benchmark guidance.
- More offline smoke fixtures for recommendation/advisor behavior.
- Stronger audit views for actions that change system state.
- Clearer separation between observation, recommendation, and application.

## Non-goals For Now

- No broad privileged auto-optimizer is enabled by default. Any automatic action
  must remain gated by safety class, preflight, audit logging, verification,
  rollback, and explicit policy.
- No silent system-wide tuning.
- No new low-level probes unless they answer a specific evidence gap.
- No claim that a recommendation proves root cause.

## Transitional migration markers

A `// Transitional` marker is allowed only for active migration work. Do not add
empty namespace placeholders or zero-field marker structs simply to reserve a
future module name.

Acceptable transitional modules must have:

- a local reason,
- an `Exit:` condition near the marker,
- an allowlist entry with reason and exit criteria,
- meaningful code or re-exports that are actively used,
- a plan to remove the marker.

The `MAX_MIGRATION_MARKER_MODULES` ceiling is a ratchet. When transitional
modules are removed, lower the ceiling in the same patch.
