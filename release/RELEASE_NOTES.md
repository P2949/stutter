# Supervisor release notes

Version: supervisor-hand-off
Release tag: `fyp-report-final-v2`
Date: 2026-06-09

Contents:

- `FYP_SUPERVISOR_PITCH.pdf` — supervisor pitch PDF
- `KCD1_EXPERIMENT_REPORT.pdf` — KCD1 case-study report
- `evidence-bundle.tar.gz` — curated KCD1 evidence bundle
- `ASSETS_SHA256SUMS` — checksums for individual release assets
- `SHA256SUMS` — checksum for the assembled release archive

Notes:

- This release is intended for first supervisor contact.
- The proposed assessed FYP scope is CPU-affinity/process-placement validation,
  profile explainability, counterbalanced repeated A/B benchmarking,
  uncertainty-aware reporting, and conservative recommendation verdicts.
- The KCD1 result is preliminary non-validation evidence, not a universal claim
  about KCD1 or CPU affinity.
- The full raw archive remains in the repository for audit/reproduction, but the
  lightweight release assets are the intended first-review materials.

To verify after extraction:

```bash
sha256sum -c ASSETS_SHA256SUMS
tar -xzf evidence-bundle.tar.gz
cd evidence-bundle
sha256sum -c MANIFEST.sha256
```
