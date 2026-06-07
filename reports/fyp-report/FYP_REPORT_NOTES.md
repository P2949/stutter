# FYP Report Planning Notes

> **Internal planning scaffolding - not part of the FYP submission.**
> This file is an evidence checklist for drafting `FYP_REPORT.md`, not a polished
> report or supervisor-facing document.

These notes keep the FYP report grounded in evidence before prose polish. The
main report is `FYP_REPORT.md`.

## Core Framing

Claim:
- The project is an evidence-based Linux game-performance tuning tool,
  evaluated through a real KCD1/Proton case study.
- KCD1 is the central evaluation chapter, not the whole project.
- The strongest result is that `stutter` avoided recommending a plausible but
  unsupported CPU-affinity profile.

Evidence:
- `reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md`
- `reports/kcd1-case-study/CASE_STUDY_SUMMARY.md`
- `docs/TUNING_WORKFLOW.md`
- `docs/SAFETY.md`
- `docs/FULL_SYSTEM_WATCHER_ARCHITECTURE.md`

## Abstract

Claims:
- Linux game tuning is noisy and often anecdotal.
- `stutter` collects scheduler, frame, process-tree, GPU, and quality evidence.
- It generates scoped tuning hypotheses and validates them through repeated
  measurement.
- KCD1 produced a negative tuning result but a positive workflow validation.

Evidence:
- KCD1 A/B summary selected `baseline-online`.
- `tuning_summary.json` profile statistics show the tuned profile worse on the
  primary diagnostic score in all five paired iterations.

## Introduction

Claims:
- Average FPS does not capture stutter, p99 frametime, or runnable latency.
- Proton/Wine/Gamescope workloads are multi-process and noisy.
- Manual tuning needs evidence, comparison, and restraint.
- Research question: Can a Linux game-performance tool collect enough evidence
  from a real Proton workload to generate, test, and validate or decline to
  recommend a tuning hypothesis?

Evidence:
- README runnable-latency definition.
- KCD1 noisy baseline medians and p99s.
- Tuning workflow verdict model.

## Background

Claims:
- Runnable latency is the delay between wakeup and actual CPU execution.
- Wine/Proton games create task trees that are more complex than one process.
- Gamescope and MangoHud make frame capture and presentation context visible.
- Repeated A/B testing is needed because real game runs vary.

Evidence:
- `README.md`
- `docs/TUNING_WORKFLOW.md`
- KCD1 baseline tables.

## Requirements

Functional:
- Record runtime evidence from a target process tree.
- Correlate scheduler and frame timing evidence.
- Classify relevant tasks and processes.
- Generate explicit tuning hypotheses.
- Apply supported low/medium-risk actions only through policy-controlled paths,
  with preflight checks and rollback requirements where applicable.
- Compare baseline and tuned runs.
- Report uncertainty, quality, and verdicts.

Non-functional:
- Safety first.
- Explicit targeting.
- Reversibility where applicable.
- No false confidence.
- Reproducible artifacts.
- Human-auditable recommendations.

Evidence:
- `docs/SAFETY.md`
- `docs/FULL_SYSTEM_WATCHER_ARCHITECTURE.md`
- `docs/AUTOTUNE_ARCHITECTURE.md`

## Architecture

Claims:
- The main pipeline is `record -> analyze/report -> advisor -> profile/fix plan
  -> tune/recommend -> explain`.
- Observation and planning do not mutate the machine.
- Mutation is gated by `TuningAction`, action runners, `DaemonPolicy`, rollback,
  and audit.

Evidence:
- `docs/FULL_SYSTEM_WATCHER_ARCHITECTURE.md`
- `docs/AUTOTUNE_ARCHITECTURE.md`
- `docs/SAFETY.md`

## Implementation

Claims:
- The project is a Rust workspace with crates for core primitives, config,
  eBPF, reporting, and the main CLI.
- eBPF captures runnable latency.
- JSON/NDJSON artifacts support reproducibility.
- The scoring model combines scheduler and frame pacing signals for comparable
  runs.
- The validation corpus and architecture tests are part of the engineering
  evidence.

Evidence:
- `Cargo.toml`
- `README.md`
- `stutter/tests/fixtures/runs/**`
- `xtask fixture-check`

## Validation Methodology

Claims:
- Repeated measurements are required.
- `baseline-online` is the within-tune comparison control.
- Warmup and measurement windows separate stabilization from scoring.
- `diagnostic_raw_score_total` is an internal comparison score, not FPS.
- `NeedsRetest`, underpowered, inconclusive, and invalid-experiment verdicts
  prevent false confidence.

Evidence:
- `docs/TUNING_WORKFLOW.md`
- KCD1 tuning summary and recommendation artifacts.

## KCD1 Case Study

Claims:
- Real KCD1/Proton workload captured successfully.
- Baselines were valid but noisy.
- Advisor generated a plausible CPU-affinity hypothesis.
- Profile explainability later showed important KCD1 threads were matched.
- A/B testing did not validate the tuned profile.
- The result validates the workflow rather than the tuning tweak.

Evidence:
- `reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md`
- `reports/kcd1-case-study/tune/kcd1-affinity-02/tuning_summary.json`
- `reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan-summary.json`
- `reports/kcd1-case-study/drop-counter-pilot/mapfactor-4-comparison.txt`
- `reports/kcd1-case-study/realworld-stack/realworld-stack-summary.csv`

## Discussion

Claims:
- A negative tuning result can be a positive validation result.
- The tool handled a complex real workload and avoided a false-positive
  recommendation.
- Explainability made the A/B result easier to audit.
- The evaluation demonstrates method, not a general KCD1 optimization.

Evidence:
- KCD1 A/B profile comparison.
- Profile-plan follow-up.
- Tuning recommendation `NeedsRetest`.

## Limitations

Claims:
- One machine, one game, one route, one Proton version.
- Formal KCD1 artifacts did not include IRQ/KMS/DRM correlation.
- Five runs per profile are enough for demonstration but not precise
  small-effect estimates.
- Personal-stack comparison is exploratory and non-causal.
- Current tool still expects technical users.

Evidence:
- KCD1 report limitations.
- KCD1 uncertainty estimates of roughly 18-30+ runs per condition for some
  metrics.

## Future Work

Claims:
- Expand IRQ/KMS/DRM capture in future case studies.
- Evaluate more games and hardware.
- Add stronger sample-size guidance and report generation.
- Improve profile explainability and foreground/target selection.
- Consider a UI/dashboard after the core evidence loop is stable.

Evidence:
- KCD1 limitations and future-work sections.
- `docs/REPORT_VIEW_ROADMAP.md`

## Appendices

Include:
- Command shapes.
- KCD1 profile TOML.
- Artifact map.
- Reproducibility checklist.
- Glossary.

## References Status

External references have been added to `FYP_REPORT.md` for eBPF, Linux
scheduling, Proton/Wine/DXVK/Gamescope, MangoHud, Mesa/Vulkan, and measurement
methodology.

The final report uses a consistent IEEE-style numeric web-reference format with
uniform `Jun. 5, 2026` access dates. For future edits, preserve inline
references on background claims and add methodology references for any new
discussion of bootstrap, sample-size estimation, or statistical power.

## Evidence Matrix

| Claim | Evidence |
| --- | --- |
| Five formal baselines were valid | `reports/kcd1-case-study/runs/baseline-*-analysis.json`, postcheck files |
| Frames were ingested | `reports/kcd1-case-study/mangohud/baseline-*.csv`, `frame_correlation.json` artifacts |
| Timestamp alignment was monotonic | `reports/kcd1-case-study/CASE_STUDY_SUMMARY.md`, baseline analysis artifacts |
| Profile matched key KCD threads | `reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan-summary.json`, full profile-plan JSON |
| `baseline-online` had lower score in all A/B iterations | `reports/kcd1-case-study/tune/kcd1-affinity-02/tuning_summary.json` |
| Recommendation was `NeedsRetest` | `reports/kcd1-case-study/tune/kcd1-affinity-02/tuning_recommendation.json` |
| Drop counter was not ringbuf failure | `reports/kcd1-case-study/drop-counter-pilot/mapfactor-4-comparison.txt` |
| Personal stack is exploratory | `reports/kcd1-case-study/realworld-stack/README.md`, `reports/kcd1-case-study/realworld-stack/setup/launch-options.md` |
| Build/test flow exists | `reports/kcd1-case-study/setup/build-check.txt`, validation commands in `FYP_REPORT.md` |
