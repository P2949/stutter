#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"
cd "${REPO_ROOT}"

SOAK_SECONDS="${SOAK_SECONDS:-300}"
OUT_DIR="${OUT_DIR:-${REPO_ROOT}/target/smoke/autotune-fake-apply}"
LOG_FILE="${LOG_FILE:-${OUT_DIR}/cargo-test.log}"

mkdir -p "${OUT_DIR}"
: >"${LOG_FILE}"

if [[ "${EUID}" == "0" && "${ALLOW_ROOT:-0}" != "1" ]]; then
    echo "autotune fake apply smoke is intentionally rootless; refusing to run as root unless ALLOW_ROOT=1" >&2
    exit 2
fi

DEADLINE=$((SECONDS + SOAK_SECONDS))
ITERATION=0

run_test() {
    local test_name="$1"
    echo "== iteration=${ITERATION} test=${test_name} ==" | tee -a "${LOG_FILE}"
    cargo test -p stutter "${test_name}" -- --nocapture 2>&1 | tee -a "${LOG_FILE}"
}

while (( SECONDS < DEADLINE )); do
    ITERATION=$((ITERATION + 1))

    run_test fake_action_preflight_failure_blocks_apply_and_verify
    run_test fake_action_apply_failure_blocks_verify_and_rollback
    run_test fake_action_verify_failure_rolls_back_mutation
    run_test fake_action_rollback_failure_keeps_mutation_and_reports_emergency
    run_test fake_action_slow_apply_still_verifies_and_returns_rollback_token
    run_test fake_action_dry_run_never_applies
    run_test journal_applied_state_rolls_back_on_start
    run_test journal_clean_state_does_nothing
    run_test rollback_failure_enters_faulted
done

grep -F "fake_action_slow_apply_still_verifies_and_returns_rollback_token" "${LOG_FILE}" >/dev/null
grep -F "rollback_failure_enters_faulted" "${LOG_FILE}" >/dev/null

echo "autotune fake apply smoke passed"
echo "iterations=${ITERATION}"
echo "log_file=${LOG_FILE}"
