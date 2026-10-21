# Project Overview

`stutter` is an experimental open-source Linux scheduler runnable-latency
profiler and evidence-based tuning toolkit built with Rust and Aya eBPF.

The project is designed for technical users who want to investigate frame-pacing
and system-performance problems with measurements rather than apply tuning advice
blindly. Its core workflow is intentionally conservative:

```text
observe -> diagnose -> form a scoped hypothesis -> explain the proposed change
-> test it repeatedly -> recommend, reject, or request more evidence
```

A successful run does not need to end with a tuning recommendation. A negative or
uncertain result is useful when it prevents a plausible tweak from being promoted
without enough evidence.

## Problem model

Linux game performance is shaped by several interacting layers:

```text
game
  -> Proton / Wine
  -> DXVK / Vulkan / graphics driver
  -> Gamescope or compositor
  -> Linux scheduler and kernel runtime behavior
  -> CPU, GPU, memory, storage, and display path
```

Average FPS compresses a whole run into one value and can hide uneven frame
delivery. `stutter` therefore treats frame pacing, scheduler-visible delay,
process identity, and data quality as separate pieces of evidence.

One primary scheduler signal is runnable latency:

```text
sched_wakeup timestamp -> sched_switch timestamp = runnable latency
```

Runnable latency measures how long a task was ready to execute before it
received CPU time. It is not proof that the scheduler caused every visible
stutter. It is evidence that can be correlated with frame timing and task
identity.

## Evidence model

A useful performance claim needs enough context to explain what was measured and
whether two runs are comparable. `stutter` therefore records or derives evidence
such as:

- scheduler wakeup and scheduling delay;
- process-tree and task identity;
- workload classification;
- MangoHud frame timing when provided;
- GPU and optional system telemetry;
- experiment configuration;
- data-quality and drop/replacement counters;
- generated reports and machine-readable artifacts.

The project keeps observation separate from interpretation. Missing evidence,
optional probe failures, or low-quality runs should reduce confidence rather than
be silently treated as successful measurements.

## Workflow

### 1. Record

Capture a workload with a stable scenario and retain the resulting artifacts.

### 2. Diagnose

Inspect scheduler, frame, process, and quality evidence to identify plausible
performance problems. Diagnosis should describe evidence and uncertainty rather
than assert a root cause that has not been validated.

### 3. Form a scoped hypothesis

Convert a diagnosis candidate into a specific, reversible experiment. The
hypothesis should state what will change, what should improve, and what evidence
would count against the change.

### 4. Explain the proposed change

Before applying a profile or action, inspect which processes, threads, classes,
rules, CPU masks, or other targets will be affected. Broad matching rules can
move more work than their names imply, so explainability is part of the safety
model.

### 5. Validate with repeated measurements

Compare baseline and candidate conditions under the same workload. Repeated A/B
measurements, warmup handling, candidate-order control, comparability checks,
uncertainty estimates, and data-quality gates are used to avoid promoting noise.

### 6. Report a conservative verdict

A candidate can be validated, rejected, invalidated by experiment quality, or
left as `NeedsRetest`. Ranking a candidate is not enough to prove a useful
effect.

## Design goals

The project should:

- collect useful scheduler and frame-pacing evidence with practical overhead;
- preserve enough context to reproduce and audit experiments;
- keep observation, recommendation, mutation, verification, and rollback
  distinct;
- expose machine-readable artifacts in addition to human-readable reports;
- explain why a recommendation was made and what it would change;
- prefer reversible experiments over persistent hidden changes;
- reject or downgrade conclusions when evidence is incomplete or experiments
  are not comparable;
- remain useful even when a tuning hypothesis fails.

## Safety model

System tuning is treated as an experiment, not as a guaranteed optimization.

Supported actions should use the project safety path where applicable:

```text
preflight -> dry-run -> apply -> verify -> audit -> rollback
```

The exact policy depends on action class. Some actions are low risk, some require
explicit policy or user intent, and some remain suggest-only. Persistent changes
must not be hidden from the user.

Data quality is also a safety concern. A tuning result should not be promoted
when the experiment itself is invalid, underpowered, non-comparable, or missing
evidence required by its objective.

See `docs/SAFETY.md`, `docs/AUTOTUNE_SAFETY.md`, and
`docs/DEGRADED_EVIDENCE.md` for the detailed rules.

## Architecture

The workspace is split so stable shared concerns can move out of the main
runtime crate without forcing runtime-specific state into reusable libraries.

Important workspace areas include:

- `stutter/`: main CLI, recorder, analysis, report integration, tuning,
  recommendation, daemon, and runtime behavior;
- `stutter-ebpf/`: eBPF-side event collection;
- `stutter-common/`: shared ABI/common types used across the eBPF boundary;
- `stutter-core/`: stable shared core types and identifiers;
- `stutter-config/`: shared configuration primitives and static validation;
- `stutter-report/`: migrated report-domain models and reusable rendering /
  analysis functionality;
- `xtask/`: repository validation, fixture, dependency, architecture, and
  maintenance commands.

The exact ownership boundary is documented in
`docs/ARCHITECTURE_BOUNDARIES.md`,
`docs/SUBSYSTEM_OWNER_CONTRACTS.md`,
`docs/CONFIG_CRATE_MIGRATION.md`, and
`docs/REPORT_CRATE_MIGRATION.md`.

## Experimental status

`stutter` is intentionally experimental. Some daemon and autotune infrastructure
exists in-tree before it is considered ready for broad user-facing enablement.
Roadmap status describes supported maturity, not whether prototype code exists.

Interfaces, artifact schemas, tuning behavior, supported actions, and report
formats may evolve as the measurement and safety model improves.

## Non-goals

The project does not assume that:

- every stutter is scheduler-caused;
- a higher average FPS automatically means better frame pacing;
- a plausible Linux tweak is a validated recommendation;
- one successful result generalizes across games, hardware, kernels, drivers, or
  Proton versions;
- automatic tuning should be enabled broadly by default.

## Real-world case studies

`reports/kcd1-case-study/` contains the current real-world case study. It is
valuable specifically because the tested CPU-affinity profile was plausible but
was not validated by the available measurements. The result demonstrates the
expected workflow behavior: retain the evidence, expose the limitations, and do
not turn an unsupported hypothesis into advice.

The large raw KCD1 capture archive is distributed separately from the source
repository. The normal clone keeps the report, summary, curated evidence, and
artifact index.

## Related documentation

- `README.md`: build, install, CLI, recording, reporting, and tuning usage.
- `docs/EXPERIMENTAL_METHODOLOGY.md`: controlled experiment and interpretation
  rules.
- `docs/TUNING_WORKFLOW.md`: tuning and recommendation workflow.
- `docs/SAFETY.md`: project safety model.
- `docs/ARTIFACT_SCHEMA.md`: persisted artifact contracts.
- `docs/VALIDATION_CORPUS.md`: regression and compatibility fixtures.
- `docs/ROADMAP.md`: current project direction and maturity.
- `CONTRIBUTING.md`: development and validation workflow.
