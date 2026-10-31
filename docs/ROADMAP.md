# Roadmap

`stutter` is moving from profiler toward trustworthy recommender in small, auditable steps.

## Project status

Current state: **v0.2.0 is released**
([releases/v0.2.0.md](releases/v0.2.0.md)). Core Validation is complete and `v0.1.0` is released
([releases/v0.1.0.md](releases/v0.1.0.md)); the staged plan is in
[CORE_VALIDATION_PLAN.md](CORE_VALIDATION_PLAN.md) and the historical ledger in
[CORE_VALIDATION_STATUS.md](CORE_VALIDATION_STATUS.md). The Post-v0.1 Evidence Hardening phase
(diagnosis corpus v2 and recalibration, the StutterScore workload matrix, release-publication
hardening, persistent multi-kernel resume on NixOS) and the autonomous-graduation milestones
M3 to M9 are complete. No weight or threshold was changed to obtain any result below.

**Supported autonomous boundary** (exactly the entries of `AUTONOMOUS_CONTRACTS`, enforced at
runtime; everything absent from it is suggest-only, benchmark-only, manual or rejected):

- CPU-affinity profile and cgroup placement, each for a stable, explicitly identified game
  under scheduler pressure judged by `GameRunnableLatency`;
- the **bounded autonomous controller** (`bounded_autotune_controller`) that applies them. It
  graduated on a coverage-qualified real-system soak (M9, below) and authorises nothing beyond
  those two contracts.

**What the milestones concluded** (details in
[MUTATION_GRADUATION_STATUS.md](MUTATION_GRADUATION_STATUS.md)):

- M3 control model v2: single objective authority, typed experiment contract, runtime objective
  and capability gates. M4 and M6: CPU affinity and cgroup placement Supported.
- M5 prepared (journal-before-write) two-phase privileged transaction.
- Evaluated and left **DeferredEvidence / Experimental**, with their evidence kept: uclamp, nice,
  I/O priority (M6); CPU EPP (M7); GPU power profile, VM swappiness, known-device IRQ affinity
  (M8). Governor, forced GPU DPM, dirty ratios, storage/unknown IRQs and arbitrary sysfs stay
  manual/HighRisk; `HIGH_RISK_APPLY_IMPLEMENTED` remains `false`.
- M9: the bounded controller passed a coverage-driven qualification at one exact commit: a
  minimum 8 h dwell, at least 6 complete clean schedule cycles (7 achieved, 113 of 113 rounds
  clean over 8.03 h), at least 6 clean executions of every dangerous event kind (controller
  crash and stop, worker kill, suspend/resume, workload kill, focus swap, long rounds), and
  repeated kept and reverted experiments of both operations, with all five zero-violation checks.
  It was backed by a 10,000-seed deterministic fault sweep and a real-controller SIGINT stress in
  `ci-live`; both soaks that preceded it found real defects (a prepared-apply race and a lost
  SIGINT), which were fixed and invalidated those runs
  ([AUTOTUNE_SOAK.md](AUTOTUNE_SOAK.md)).

**Still Experimental:** the daemon/service integration (`always_on_autotune`; the packaged
service runs `apply-low-risk`, which the soak does not exercise), every system-wide mutation,
the deferred process-local operations above, and remote mutation.

Earlier results (all Experimental, no weight or threshold changed):

- v0.1.0 is published as a GitHub Release with verified assets; future releases go through the
  idempotent `xtask release-publish` ([RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md)).
- Diagnosis corpus v2 and recalibration: [DIAGNOSIS_CALIBRATION_V2.md](DIAGNOSIS_CALIBRATION_V2.md).
- StutterScore across a workload matrix: [SCORE_EVALUATION_V2.md](SCORE_EVALUATION_V2.md); it
  stays an Experimental composite, and autonomy no longer depends on it (issue #129).
- Persistent autonomous multi-kernel resume on NixOS is proven end to end
  ([VALIDATION_CAMPAIGN.md](VALIDATION_CAMPAIGN.md)).

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
| Limited auto-tuner | Supported (bounded) | The bounded autonomous controller applies CPU-affinity and cgroup-placement experiments for its exact registry contracts, with prepared (journal-before-write) transactions, physical verification, rollback, audit and crash recovery; qualified by the M9 soak. Enablement of anything outside `AUTONOMOUS_CONTRACTS` stays gated. |
| Broader optimizer | Experimental / deferred | Providers and candidate paths exist for nice, ioprio, uclamp, IRQ affinity, CPU power, VM knobs, and GPU power, each evaluated individually and left DeferredEvidence/Experimental; the daemon/service always-on integration is also Experimental. Promotion needs new evidence for each. |

### Autotune Implementation Status

The autotune subsystem is partially implemented in-tree. Roadmap status refers to
safe user-facing enablement, not absence of code. Experimental/internal paths may
exist before they are considered supported defaults.

### Objective comparison maturity

Autotune control model v2 (M3): the candidate plan is the single authority for its objective;
objective maturity and product-capability tiers are runtime gates for normal autonomous apply
(see [AUTOTUNE_CONFIG.md](AUTOTUNE_CONFIG.md)); nothing is autonomous until an operation
completes graduation.

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
