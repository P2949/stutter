# Supervisor release notes

Version: supervisor-hand-off-v4
Release tag: `fyp-report-final-v4`
Date: 2026-06-09
Asset refresh: 2026-06-12

First-review checklist:

1. Read `FYP_SUPERVISOR_PITCH.pdf`.
2. Skim `SUPERVISOR_README.md` in the repository root.
3. Skim `reports/fyp-report/PRE_FYP_SCOPE_NOTE.md`.
4. Check `reports/fyp-report/AI_DISCLOSURE.md`.

The raw KCD1 archive and full source tree are for audit/reproduction only. First
contact is not intended to ask for a full code review.

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

The raw KCD1 archive files listed above are GitHub Release assets only; they are
not included in the lightweight repository checkout or supervisor tarball.

Notes:

- This release is intended for first supervisor contact.
- The proposed assessed FYP scope is CPU-affinity/process-placement validation,
  profile explainability, counterbalanced repeated A/B benchmarking,
  uncertainty-aware reporting, and conservative recommendation verdicts.
- The KCD1 result is preliminary non-validation evidence, not a universal claim
  about KCD1 or CPU affinity.
- The full raw KCD1 archive is published as a release asset instead of being
  tracked in Git, keeping normal repository clones lightweight.
- First review should use the pitch PDF, supervisor README, scope note, AI
  disclosure, and curated evidence bundle. The raw KCD1 archive is included for
  audit/reproduction only.
- Tracked files under `release/` and generated PDFs under `reports/` are frozen
  supervisor-review snapshots. Markdown sources remain the editable source of
  truth.
- Markdown-derived PDFs are regenerated from repository Markdown sources with
  the pinned in-repo pipeline (`scripts/artifacts/build_supervisor_release.sh`,
  pandoc + Typst); no browser header/footer local path metadata is embedded.
- 2026-06-12 asset refresh: the pitch's small-effect sample-size figure was
  corrected to the 19-30 runs-per-condition range recorded in the KCD1
  tuning-recommendation evidence; the pitch and KCD1 report PDFs were
  regenerated from the corrected Markdown sources; the curated evidence
  bundle tarball and both checksum files were rebuilt. The raw KCD1 archive
  assets are unchanged.
- Before sending the email, manually confirm that the raw archive, raw archive
  checksum, contents listing, and contents-listing checksum are attached to the
  GitHub Release page.

To verify the lightweight evidence bundle after extraction:

```bash
sha256sum -c ASSETS_SHA256SUMS
tar -xzf evidence-bundle.tar.gz
cd evidence-bundle
sha256sum -c MANIFEST.sha256
```

To verify the raw KCD1 archive:

```bash
sha256sum -c raw-kcd1-case-study-fyp-report-final-v4.tar.zst.sha256
tar -tf raw-kcd1-case-study-fyp-report-final-v4.tar.zst | head
```
