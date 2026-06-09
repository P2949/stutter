# Stutter Supervisor Readme

[![CI](https://github.com/P2949/stutter/actions/workflows/ci.yml/badge.svg?branch=experimental)](https://github.com/P2949/stutter/actions/workflows/ci.yml?query=branch%3Aexperimental)

This is the curated entry point for supervisor review. `stutter` is a Rust/eBPF
prototype for evidence-based Linux/Proton frame-pacing analysis.

The proposed assessed FYP boundary is CPU-affinity/process-placement validation:
profile explanation, repeated counterbalanced A/B measurement, uncertainty-aware
reporting, and conservative recommendation verdicts.

The preliminary KCD1 A/B result is intentionally caveated: it was paired but not
counterbalanced because the online baseline was measured before the tuned profile
in every iteration, so the proposed FYP includes a corrected counterbalanced
protocol as future assessed work.

## Read First

- [FYP supervisor pitch](reports/fyp-report/FYP_SUPERVISOR_PITCH.md) - concise
  project motivation, current prototype status, preliminary KCD1 result, and
  supervision questions.
- [Pre-FYP scope note](reports/fyp-report/PRE_FYP_SCOPE_NOTE.md) - separates
  existing prototype work from the proposed assessed FYP contribution.
- [AI use disclosure](reports/fyp-report/AI_DISCLOSURE.md) -
  describes the role of AI coding assistants in the prototype workflow.
- [KCD1 case-study report](reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md) -
  polished preliminary real-game case study.
- [Curated KCD1 evidence bundle](evidence-bundle/README.md) - small artifact set
  for inspecting the non-validation result without opening the full raw archive.
- [Supervisor-review release assets](https://github.com/P2949/stutter/releases/tag/fyp-report-final) -
  PDFs and bundle assets for first contact.

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

## What To Ignore For First Review

For supervisor triage, the daemon, remote agent, broad autotune controller,
GPU/IRQ/VM actions, packaging skeleton, and live service paths should be read as
background prototype areas. They are not proposed as the assessed FYP deliverable
unless a supervisor explicitly chooses to bring one into scope.

## Large Artifacts

The full repository contains a large raw evidence archive. For first review, use
the pitch, scope note, KCD1 report, and curated evidence bundle; cloning the full
raw archive is not required. The 2026-06-07 size audit found `reports/` at
1017M, `evidence-bundle/` at 96K, and `stutter/` at 11M.

Final decision for supervisor handoff (2026-06-09):

- Retain the full raw archive in the development branch for audit and reproduction.
- For initial supervisor contact prefer the lightweight release archive (`FYP_SUPERVISOR_PITCH.pdf`, `KCD1_EXPERIMENT_REPORT.pdf`, `evidence-bundle.tar.gz`) which is checksum-verified.
- Long-term archival (Git LFS or external storage) will be arranged after supervisor guidance.

## Current Validation Checks

Last local validation on 2026-06-09T12:31:36Z for commit 8745c722b3e0939d7e3f06da1de0b17db65097b9:

- `RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- ci`
- `RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- fixture-check`
- `RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- dependency-hygiene`
- `cd evidence-bundle && sha256sum -c MANIFEST.sha256`

The main non-root validation gate is:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- ci
```

Useful additional checks for the evidence/reporting side are:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- fixture-check
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- dependency-hygiene
```

The curated evidence bundle is verified with:

```bash
cd evidence-bundle
sha256sum -c MANIFEST.sha256
```

For a supervisor demo, prefer the offline path before live eBPF tracing:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- fixture-check
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- dependency-hygiene
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p stutter -- doctor
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p stutter -- profile-plan --help
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p stutter -- tune --help
```

Normal cargo builds may emit noisy eBPF/build-script output even when validation
passes. Treat successful exit status from the pinned validation commands as the
authoritative signal.

The current case-study conclusion is deliberately conservative: the KCD1
CPU-affinity profile was plausible, but it was not validated by the preliminary
five-iteration A/B run.
