#!/usr/bin/env bash
# Offline release-candidate gates (PLAN.md section 17) from a FRESH checkout of an exact
# commit. Writes a JSON receipt; exits nonzero if any gate fails. Does not run the trusted
# real-system gates (live observation/game-runnable/affinity/recovery/suspend/kernel/overhead).
# Usage: scripts/release/offline-acceptance.sh <commit> <receipt.json> [local-repo-path]
set -uo pipefail
commit="${1:?commit}"; receipt="${2:?receipt path}"; source="${3:-.}"
# Unix-socket tests fail on long temp paths; a controller inside `nix develop` has a deep TMPDIR.
export TMPDIR=/tmp
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
git clone -q "$source" "$work/src" && git -C "$work/src" checkout -q "$commit" || { echo "cannot check out $commit"; exit 2; }
head="$(git -C "$work/src" rev-parse HEAD)"
[ -z "$(git -C "$work/src" status --porcelain)" ] || { echo "fresh checkout is dirty"; exit 2; }
# One canonical contract: every stage of `xtask ci` (listed by xtask itself), plus the checks the
# fresh-checkout receipt has always carried. Nothing here depends on a remote or a hosted service.
steps=(
  "nix flake check|nix flake check --no-build"
  "verify-fast|cargo run -q -p xtask -- verify-fast"
)
# The shell prints a banner before the command's own output: keep only stage names.
stages="$(cd "$work/src" && nix develop --command cargo run -q -p xtask -- ci --list | grep -E '^[a-z][a-z-]*$')" || { echo "cannot list the CI stages"; exit 2; }
[ -n "$stages" ] || { echo "the CI lists no stages"; exit 2; }
for stage in $stages; do
  steps+=("ci $stage|cargo run -q -p xtask -- ci --stage $stage")
done
steps+=(
  "fixture-check|cargo run -q -p xtask -- fixture-check"
  "fixture-coverage|cargo run -q -p xtask -- fixture-coverage"
  "diff check|git diff --check"
)
results=(); failed=0
for entry in "${steps[@]}"; do
  name="${entry%%|*}"; cmd="${entry#*|}"; start=$SECONDS
  if (cd "$work/src" && nix develop --command bash -c "$cmd") > "$work/step.log" 2>&1; then status=PASS; else status=FAIL; failed=1; tail -20 "$work/step.log" >&2; fi
  results+=("{\"name\":\"$name\",\"command\":\"$cmd\",\"status\":\"$status\",\"seconds\":$((SECONDS-start))}")
  echo "$status $name"
done
overall=PASS; [ "$failed" = 0 ] || overall=FAIL
printf '{"schema_version":1,"suite":"offline_release_acceptance","commit":"%s","fresh_checkout":true,"status":"%s","steps":[%s],"not_run":["live observation","live game-runnable","live affinity","live recovery","suspend/resume","kernel compatibility","profiler overhead","package install and doctor/record smoke"]}\n' \
  "$head" "$overall" "$(IFS=,; echo "${results[*]}")" > "$receipt"
echo "$overall: receipt $receipt"
[ "$overall" = PASS ]
