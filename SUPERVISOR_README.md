# Stutter Supervisor Readme

[![CI](https://github.com/P2949/stutter/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/P2949/stutter/actions/workflows/ci.yml?query=branch%3Amain)

This is the curated entry point for supervisor review. `stutter` is a Rust/eBPF
prototype for evidence-based Linux/Proton frame-pacing analysis.

The proposed assessed FYP boundary is CPU-affinity/process-placement validation:
profile explanation, repeated counterbalanced A/B measurement, uncertainty-aware
reporting, and conservative recommendation verdicts.

The preliminary KCD1 A/B result is intentionally caveated: it was paired but not
counterbalanced because the online baseline was measured before the tuned profile
in every iteration, so the proposed FYP includes a corrected counterbalanced
protocol as future assessed work.

**Current handoff status, 2026-06-09:** first-contact materials are prepared for
the `fyp-report-final-v4` supervisor release. The full raw KCD1 archive is
published as a release asset for audit/reproduction; first review should use
the pitch, scope note, KCD1 executive summary, AI disclosure, and curated
evidence bundle.

## Read First

For first review, please read only:

1. `FYP_SUPERVISOR_PITCH.pdf` from the
   [fyp-report-final-v4 release](https://github.com/P2949/stutter/releases/tag/fyp-report-final-v4).
2. `SUPERVISOR_README.md`.
3. `reports/fyp-report/PRE_FYP_SCOPE_NOTE.md`.
4. `reports/fyp-report/AI_DISCLOSURE.md`.

The raw KCD1 archive and full source tree are for audit/reproduction only. First
contact is not intended as a full code-review request; the intended first review
is the short pitch and this curated supervisor README.

For a slightly deeper triage pass, use this order:

1. Read the pitch PDF.
2. Skim the scope note.
3. Read the executive summary of `reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md`.
4. Check the AI disclosure.

- [FYP supervisor pitch](reports/fyp-report/FYP_SUPERVISOR_PITCH.md) - concise
  project motivation, current prototype status, preliminary KCD1 result, and
  supervision questions.
- [Pre-FYP scope note](reports/fyp-report/PRE_FYP_SCOPE_NOTE.md) - separates
  existing prototype work from the proposed assessed FYP contribution.
- [FYP scope map](docs/FYP_SCOPE_MAP.md) - one-page map of what exists in the
  prototype versus what is proposed as core assessed FYP scope.
- [AI use disclosure](reports/fyp-report/AI_DISCLOSURE.md) -
  describes the role of AI coding assistants in the prototype workflow.
- [KCD1 case-study report](reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md) -
  polished preliminary real-game case study.
- [Curated KCD1 evidence bundle](evidence-bundle/README.md) - small artifact set
  for inspecting the non-validation result without opening the full raw archive.
- [Supervisor-review release assets](https://github.com/P2949/stutter/releases/tag/fyp-report-final-v4) -
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

### Known caveats / not claimed

This supervisor snapshot does not claim:

- that `stutter` is a finished optimizer;
- that the KCD1 affinity profile improves performance;
- that the preliminary KCD1 A/B run is a final counterbalanced experiment;
- that GPU/IRQ/VM/daemon/autotune features are proposed core FYP scope.

## What To Ignore For First Review

For supervisor triage, the daemon, remote agent, broad autotune controller,
GPU/IRQ/VM actions, packaging skeleton, and live service paths should be read as
background prototype areas. They are not proposed as the assessed FYP deliverable
unless a supervisor explicitly chooses to bring one into scope.

## Large Artifacts

The full raw KCD1 archive is no longer tracked in Git. It is published as a
GitHub Release asset for the `fyp-report-final-v4` handoff tag.

For first review, use the pitch, scope note, KCD1 report, AI disclosure, and
curated evidence bundle. The raw archive should only be downloaded if audit or
reproduction artifacts are requested.

Final decision for supervisor handoff (2026-06-09):

- Keep the normal repository checkout lightweight.
- Retain the curated `evidence-bundle/` in Git.
- Publish the full raw KCD1 archive as
  `raw-kcd1-case-study-fyp-report-final-v4.tar.zst` on the GitHub release.
- Keep checksums and a contents listing beside the raw archive release asset.

## Current Validation Checks

Supervisor handoff target: `fyp-report-final-v4`.

The exact validated commit SHA is recorded by the Git tag and GitHub release
metadata. The release tag is the authoritative supervisor handoff target.

The local validation gate for the handoff target is:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo fmt --all -- --check
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- ci
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- fixture-check
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- dependency-hygiene
cd evidence-bundle && sha256sum -c MANIFEST.sha256
cd ../release && sha256sum -c ASSETS_SHA256SUMS
sha256sum -c SHA256SUMS
```

For a supervisor demo, prefer the offline script before live eBPF tracing:

```bash
scripts/demo/supervisor_offline_demo.sh
```

The script uses offline/safe checks first. The equivalent manual path is:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- fixture-check
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- dependency-hygiene
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p stutter -- doctor
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p stutter -- profile-plan --help
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p stutter -- tune --help
```

Normal cargo builds may emit noisy eBPF/build-script output even when validation
passes. The supervisor offline demo captures build, fixture-check, and
dependency-hygiene detail under `target/supervisor-demo/` and prints a captured
log only if that step fails. Treat successful exit status from the pinned
validation commands as the authoritative signal.

The current case-study conclusion is deliberately conservative: the KCD1
CPU-affinity profile was plausible, but it was not validated by the preliminary
five-iteration A/B run.
