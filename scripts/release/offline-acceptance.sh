#!/usr/bin/env bash
# Offline release-candidate gates (PLAN.md section 17) from a FRESH checkout of an exact
# commit. Writes a JSON receipt; exits nonzero if any gate fails. Does not run the trusted
# real-system gates (live observation/game-runnable/affinity/recovery/suspend/kernel/overhead).
# Usage: scripts/release/offline-acceptance.sh <commit> <receipt.json> [repo-url-or-path]
set -uo pipefail
commit="${1:?commit}"; receipt="${2:?receipt path}"; source="${3:-.}"
# Unix-socket tests fail on long temp paths; a controller inside `nix develop` has a deep TMPDIR.
export TMPDIR=/tmp
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
git clone -q "$source" "$work/src" && git -C "$work/src" checkout -q "$commit" || { echo "cannot check out $commit"; exit 2; }
head="$(git -C "$work/src" rev-parse HEAD)"
[ -z "$(git -C "$work/src" status --porcelain)" ] || { echo "fresh checkout is dirty"; exit 2; }
steps=(
  "nix flake check|nix flake check --no-build"
  "fmt|cargo fmt --all -- --check"
  "dependency-hygiene|cargo run -q -p xtask -- dependency-hygiene"
  "build smoke|scripts/smoke/build.sh"
  "xtask tests|cargo test -p xtask"
  "objective docs|cargo run -q -p xtask -- objective-docs --check"
  "fixture-check|cargo run -q -p xtask -- fixture-check"
  "fixture-coverage|cargo run -q -p xtask -- fixture-coverage"
  "capability-check|cargo run -q -p xtask -- capability-check"
  "no-allow-attrs|cargo run -q -p xtask -- no-allow-attrs"
  "clippy|cargo clippy -p stutter -p xtask --all-targets -- -D warnings"
  "architecture tests|cargo test -p stutter architecture_tests"
  "schema tests|cargo test -p stutter artifact_contract_tests"
  "autotune safety|scripts/check-autotune-safety.sh"
  "offline recommendation|scripts/smoke/offline_recommendation.sh"
  "advisor offline|scripts/smoke/advisor_offline.sh"
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
