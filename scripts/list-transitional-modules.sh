#!/usr/bin/env bash
set -euo pipefail

matches="$(grep -R "Transitional" -n stutter/src --include='*.rs' | sort || true)"

if [[ -z "$matches" ]]; then
    echo "transitional modules: 0"
    exit 0
fi

echo "$matches"
echo
echo "transitional marker count: $(printf '%s\n' "$matches" | wc -l)"
