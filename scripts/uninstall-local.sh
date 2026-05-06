#!/usr/bin/env bash
set -euo pipefail

prefix="${PREFIX:-$HOME/.local}"
bindir="$prefix/bin"

rm -f "$bindir/stutter"

echo "removed $bindir/stutter"
echo "state directories were not removed automatically"
echo "default run state: $HOME/.local/state/stutter"
echo "default audit log: $HOME/.local/state/stutter/audit/actions.jsonl"
