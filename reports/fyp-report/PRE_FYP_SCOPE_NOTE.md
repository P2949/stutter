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

This prototype was developed with AI coding assistant support, including
Claude, ChatGPT/OpenAI tooling, and Codex-style review workflows where
applicable. These tools were used as iterative review and pair-programming
support: to critique code, suggest implementation alternatives, identify bugs,
draft patches for review, and help refine documentation.

All design decisions, accepted code changes, experimental setup, benchmark
execution, validation runs, interpretation of results, and final project
conclusions remain the responsibility of the author. AI-generated or
AI-suggested material was reviewed, adapted, tested, and either accepted or
rejected by the author before inclusion.

The assessed FYP contribution is proposed as the author's methodology,
implementation, validation, and reporting work. This disclosure is included to
make the nature and extent of AI assistance explicit for supervisor review and
to align with UL academic-integrity expectations.
