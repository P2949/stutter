#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

EXPECTED_PANDOC="3.6.4"
EXPECTED_TYPST="0.13.1"

pandoc_version="$(pandoc --version | head -1 | awk '{print $2}')"
typst_version="$(typst --version | awk '{print $2}')"

if [ "$pandoc_version" != "$EXPECTED_PANDOC" ] || [ "$typst_version" != "$EXPECTED_TYPST" ]; then
  echo "error: PDF tool versions do not match the pinned pipeline" >&2
  echo "  pandoc: have $pandoc_version, expected $EXPECTED_PANDOC" >&2
  echo "  typst:  have $typst_version, expected $EXPECTED_TYPST" >&2
  exit 1
fi

pandoc reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md \
  -o reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.pdf \
  --pdf-engine=typst \
  -V mainfont="DejaVu Serif" \
  -V fontsize=11pt \
  -V papersize=a4 \
  -V margin-x=2.2cm \
  -V margin-y=2.4cm \
  --include-in-header=scripts/artifacts/pdf-tables.typ

echo "Rendered reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.pdf"
