# Supervisor release notes

Version: supervisor-hand-off-v4
Release tag: `fyp-report-final-v4`
Date: 2026-06-09

Release assets:

- `FYP_SUPERVISOR_PITCH.pdf` — supervisor pitch PDF.
- `KCD1_EXPERIMENT_REPORT.pdf` — KCD1 case-study report.
- `AI_DISCLOSURE.md` — AI coding-assistant disclosure for supervisor review.
- `evidence-bundle.tar.gz` — curated KCD1 evidence bundle.
- `ASSETS_SHA256SUMS` — checksums for the lightweight supervisor-review assets.
- `SHA256SUMS` — checksums for the release assets stored in `release/`.
- `stutter-supervisor-release-fyp-report-final-v4.tar.gz` — assembled lightweight supervisor-review archive.
- `raw-kcd1-case-study-fyp-report-final-v4.tar.zst` — full raw KCD1 case-study archive for audit/reproduction.
- `raw-kcd1-case-study-fyp-report-final-v4.tar.zst.sha256` — checksum for the raw KCD1 archive.
- `raw-kcd1-case-study-fyp-report-final-v4.contents.txt` — file listing for the raw KCD1 archive.
- `raw-kcd1-case-study-fyp-report-final-v4.contents.txt.sha256` — checksum for the raw archive contents listing.

Notes:

- This release is intended for first supervisor contact.
- The proposed assessed FYP scope is CPU-affinity/process-placement validation,
  profile explainability, counterbalanced repeated A/B benchmarking,
  uncertainty-aware reporting, and conservative recommendation verdicts.
- The KCD1 result is preliminary non-validation evidence, not a universal claim
  about KCD1 or CPU affinity.
- The full raw KCD1 archive is published as a release asset instead of being
  tracked in Git, keeping normal repository clones lightweight.
- First review should use the pitch PDF, KCD1 report, AI disclosure, and curated
  evidence bundle. The raw KCD1 archive is included for audit/reproduction only.

To verify the lightweight evidence bundle after extraction:

```bash
sha256sum -c ASSETS_SHA256SUMS
tar -xzf evidence-bundle.tar.gz
cd evidence-bundle
sha256sum -c MANIFEST.sha256
````

To verify the raw KCD1 archive:

```bash
sha256sum -c raw-kcd1-case-study-fyp-report-final-v4.tar.zst.sha256
tar -tf raw-kcd1-case-study-fyp-report-final-v4.tar.zst | head
```

