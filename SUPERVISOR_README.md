# Stutter Supervisor Readme

[![CI](https://github.com/P2949/stutter/actions/workflows/ci.yml/badge.svg?branch=experimental)](https://github.com/P2949/stutter/actions/workflows/ci.yml?query=branch%3Aexperimental)

This is the curated entry point for supervisor review. The full repository is a
large prototype and evidence archive; start with the short documents below.

## Read First

- [FYP supervisor pitch](reports/fyp-report/FYP_SUPERVISOR_PITCH.md) - concise
  project motivation, current prototype status, preliminary KCD1 result, and
  supervision questions.
- [Pre-FYP scope note](reports/fyp-report/PRE_FYP_SCOPE_NOTE.md) - separates
  existing prototype work from the proposed assessed FYP contribution.
- [KCD1 case-study report](reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md) -
  polished preliminary real-game case study.
- [Curated KCD1 evidence bundle](evidence-bundle/README.md) - small artifact set
  for inspecting the non-validation result without opening the full raw archive.

## Proposed Assessment Boundary

The proposed assessed FYP is not a general Linux optimizer and not a claim that
the existing prototype is already the final project. The core assessed work is a
bounded methodology for Linux/Proton game-performance tuning validation:

- CPU-affinity and process/thread-placement hypotheses.
- Profile explanation before tuning.
- Counterbalanced repeated A/B benchmarking.
- Uncertainty-aware reporting.
- Conservative verdicts: validated improvement, regression, inconclusive,
  unsupported, or invalid experiment.

Existing daemon paths, remote/agent work, GPU power tuning, IRQ affinity, VM or
kernel tuning, scheduler replacement, packaging, and wide hardware/game coverage
are background prototype areas, optional extensions, or future work unless the
supervisor explicitly chooses to include one of them.

## Current Validation Checks

The main non-root validation gate is:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- ci
```

Useful additional checks for the evidence/reporting side are:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- fixture-check
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- dependency-hygiene
```

The current case-study conclusion is deliberately conservative: the KCD1
CPU-affinity profile was plausible, but it was not validated by the preliminary
five-iteration A/B run.
