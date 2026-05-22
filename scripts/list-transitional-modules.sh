#!/usr/bin/env bash
set -euo pipefail

matches="$(grep -R "Transitional" -n stutter/src --include='*.rs' | sort || true)"
ceiling="$(grep -E 'MAX_MIGRATION_MARKER_MODULES: usize = [0-9]+' \
  stutter/src/architecture_tests/transitional_allowlist.rs \
  | sed -E 's/.*= ([0-9]+);/\1/')"

if [[ -z "$matches" ]]; then
    echo "transitional modules: 0"
    echo "transitional ceiling: ${ceiling:-unknown}"
    exit 0
fi

echo "$matches"
echo

count="$(printf '%s\n' "$matches" | cut -d: -f1 | uniq | wc -l | tr -d ' ')"
echo "transitional marker count: $count"
echo "transitional marker ceiling: ${ceiling:-unknown}"

if [[ -n "${ceiling:-}" && "$count" -gt "$ceiling" ]]; then
    echo "error: transitional marker count exceeds ceiling" >&2
    exit 1
fi
