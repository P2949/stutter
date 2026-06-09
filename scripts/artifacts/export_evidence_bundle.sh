#!/usr/bin/env bash
set -euo pipefail

OUT_DIR="/tmp/stutter-evidence-export-$(date +%s)"
mkdir -p "$OUT_DIR"

echo "Creating evidence bundle export at $OUT_DIR"

# Copy evidence-bundle but exclude known large run traces by default
rsync -a --exclude='**/runs/**' --exclude='**/*.tar.zst' --exclude='**/*.zst' evidence-bundle/ "$OUT_DIR/evidence-bundle/"

echo "Generating checksums"
(
  cd "$OUT_DIR/evidence-bundle"
  sha256sum * > MANIFEST.sha256 || true
)

echo "Running basic privacy scan"
rg -n "/home/|/root/|p2949|Desktop|@gmail.com" "$OUT_DIR" || echo "No obvious local paths found"

echo "Export prepared: $OUT_DIR"

exit 0
