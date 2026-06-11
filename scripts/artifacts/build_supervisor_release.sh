#!/usr/bin/env bash
# Rebuild the supervisor-facing PDFs and the release/ asset set from the
# repository Markdown sources of truth.
#
# Pipeline: pandoc (Markdown -> Typst) + Typst (Typst -> PDF).
# Pinned tool versions are checked below so regenerated assets stay
# reproducible across machines. Install both as single static binaries:
#   pandoc: https://github.com/jgm/pandoc/releases (3.6.4)
#   typst:  https://github.com/typst/typst/releases (0.13.1)
#
# Steps:
#   1. Render the pitch, KCD1 report, and long-form report PDFs.
#   2. Copy the supervisor-review assets into release/.
#   3. Rebuild release/evidence-bundle.tar.gz from evidence-bundle/.
#   4. Rebuild the assembled supervisor-review tarball.
#   5. Regenerate ASSETS_SHA256SUMS and SHA256SUMS and verify them.
#
# RELEASE_NOTES.md is edited by hand; this script only checksums it.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

EXPECTED_PANDOC="3.6.4"
EXPECTED_TYPST="0.13.1"

pandoc_version="$(pandoc --version | head -1 | awk '{print $2}')"
typst_version="$(typst --version | awk '{print $2}')"
if [ "$pandoc_version" != "$EXPECTED_PANDOC" ] || [ "$typst_version" != "$EXPECTED_TYPST" ]; then
  echo "warning: tool versions differ from the pinned pipeline" >&2
  echo "  pandoc: have $pandoc_version, pinned $EXPECTED_PANDOC" >&2
  echo "  typst:  have $typst_version, pinned $EXPECTED_TYPST" >&2
  echo "  PDFs may differ byte-for-byte from previously published assets." >&2
fi

render_pdf() {
  local src="$1" out="$2"
  pandoc "$src" -o "$out" \
    --pdf-engine=typst \
    -V mainfont="DejaVu Serif" \
    -V fontsize=11pt \
    -V papersize=a4 \
    -V margin-x=2.2cm \
    -V margin-y=2.4cm \
    --include-in-header=scripts/artifacts/supervisor-pdf-tables.typ
  echo "rendered $out"
}

echo "== 1. Render PDFs from Markdown sources =="
render_pdf reports/fyp-report/FYP_SUPERVISOR_PITCH.md reports/fyp-report/FYP_SUPERVISOR_PITCH.pdf
render_pdf reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.pdf
render_pdf reports/fyp-report/FYP_REPORT.md reports/fyp-report/FYP_REPORT.pdf

echo "== 2. Copy supervisor-review assets into release/ =="
cp reports/fyp-report/FYP_SUPERVISOR_PITCH.pdf release/FYP_SUPERVISOR_PITCH.pdf
cp reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.pdf release/KCD1_EXPERIMENT_REPORT.pdf
cp reports/fyp-report/AI_DISCLOSURE.md release/AI_DISCLOSURE.md

echo "== 3. Rebuild curated evidence bundle tarball =="
(cd evidence-bundle && sha256sum -c MANIFEST.sha256 --quiet)
tar -czf release/evidence-bundle.tar.gz \
  --sort=name --owner=0 --group=0 --numeric-owner --mtime='UTC 2026-01-01' \
  evidence-bundle/

echo "== 4. Rebuild assembled supervisor-review tarball =="
(
  cd release
  sha256sum \
    FYP_SUPERVISOR_PITCH.pdf \
    KCD1_EXPERIMENT_REPORT.pdf \
    AI_DISCLOSURE.md \
    evidence-bundle.tar.gz \
    RELEASE_NOTES.md \
    > ASSETS_SHA256SUMS
  tar -czf stutter-supervisor-release-fyp-report-final-v4.tar.gz \
    --sort=name --owner=0 --group=0 --numeric-owner --mtime='UTC 2026-01-01' \
    FYP_SUPERVISOR_PITCH.pdf \
    KCD1_EXPERIMENT_REPORT.pdf \
    AI_DISCLOSURE.md \
    evidence-bundle.tar.gz \
    ASSETS_SHA256SUMS \
    RELEASE_NOTES.md
)

echo "== 5. Regenerate and verify release checksums =="
(
  cd release
  sha256sum \
    FYP_SUPERVISOR_PITCH.pdf \
    KCD1_EXPERIMENT_REPORT.pdf \
    AI_DISCLOSURE.md \
    evidence-bundle.tar.gz \
    stutter-supervisor-release-fyp-report-final-v4.tar.gz \
    ASSETS_SHA256SUMS \
    RELEASE_NOTES.md \
    > SHA256SUMS
  sha256sum -c ASSETS_SHA256SUMS
  sha256sum -c SHA256SUMS
)

echo "Supervisor release assets rebuilt successfully."
