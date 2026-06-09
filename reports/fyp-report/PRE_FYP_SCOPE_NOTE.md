# Pre-FYP Scope Note

## Existing Preliminary Work

- Working `stutter` Rust/eBPF prototype.
- Preliminary KCD1 case study.
- Supervisor-review release snapshot.
- Existing support for recording, profile planning, tuning, reporting, safety, and artifact generation.

## Proposed Assessed FYP Contribution

- Formal benchmark protocol for CPU-affinity/process-placement validation.
- Profile-plan explanation as a required pre-tuning step.
- Counterbalanced A/B methodology.
- Uncertainty-aware reporting using raw frame metrics, scheduler metrics, effect sizes, and confidence intervals.
- One primary real-game evaluation under the final protocol.
- Conservative recommendation verdicts: validated, regression, inconclusive, invalid experiment.

## Planned validation protocol

For each benchmarked workload, the FYP method should select a fixed route, warm
shader caches, record baseline evidence, run diagnosis, generate a candidate
profile, run `profile-plan` before applying it, confirm intended task coverage,
pre-register the candidate list, then collect repeated A/B or A/B/A data with
counterbalanced order.

The minimum repeat policy is at least three valid repetitions per profile for a
pilot and at least five valid repetitions per profile for the main case study,
with eight to ten repetitions per profile used if time allows. Failed
data-quality checks should trigger reruns rather than silent inclusion.

Primary metrics should include the diagnostic score used by the tuning output,
p95 or p99 frame time, over-threshold frame counts, and scheduler-visible
delay/spike indicators. Secondary metrics may include median frame time, 1% low
style summaries where available, task-class scheduler delay, MangoHud frame
pacing outliers, eBPF drop counters, and data-quality warnings.

Decision vocabulary is deliberately conservative: `validated`, `not validated`,
`inconclusive`, `more evidence required`, or `unsafe/not applicable`. The
archived KCD1 run was paired but not counterbalanced, so it remains preliminary
motivation for the corrected protocol rather than a validated tuning claim.

## Out of Core Scope

These may exist in the repository as background prototype or future-work areas,
but they are not proposed as the assessed FYP core unless supervision explicitly
changes the scope.

- General Linux game optimizer.
- Broad autotuning platform.
- Persistent daemon/service as assessed contribution.
- Remote agent.
- GPU power tuning.
- IRQ affinity.
- VM/kernel tuning.
- Scheduler replacement.
- Wide hardware/game coverage.

## Development tools and academic integrity

AI coding-assistant use during prototype development is disclosed separately in
[`AI_DISCLOSURE.md`](AI_DISCLOSURE.md). The assessed FYP contribution remains the
author's methodology, implementation, validation, and reporting work.
