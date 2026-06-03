#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

prefix="$(mktemp -d)"
trap 'rm -rf "$prefix"' EXIT

scripts/install-local.sh --prefix "$prefix"
"$prefix/bin/stutter" --version
"$prefix/bin/stutter" service doctor --manager systemd-system --mode system-observe --json >/dev/null
"$prefix/bin/stutter" daemon emergency-restore --dry-run >/dev/null

echo "local install smoke ok: $prefix"
