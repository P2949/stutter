#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"
cd "${REPO_ROOT}"

SOAK_SECONDS="${SOAK_SECONDS:-300}"
SUMMARY_MS="${SUMMARY_MS:-1000}"
PRESET="${PRESET:-diagnosis}"
OUT_DIR="${OUT_DIR:-${REPO_ROOT}/target/smoke/autotune-observe}"
DECISION_LOG="${DECISION_LOG:-${OUT_DIR}/decision-log.jsonl}"
STDOUT_LOG="${STDOUT_LOG:-${OUT_DIR}/stdout.log}"
STDERR_LOG="${STDERR_LOG:-${OUT_DIR}/stderr.log}"

mkdir -p "${OUT_DIR}"
rm -f "${DECISION_LOG}" "${STDOUT_LOG}" "${STDERR_LOG}"

if [[ -n "${STUTTER_BIN:-}" ]]; then
    STUTTER="${STUTTER_BIN}"
else
    cargo build -p stutter --bin stutter
    STUTTER="${REPO_ROOT}/target/debug/stutter"
fi

TARGET_PID_CREATED=0
if [[ -n "${STUTTER_TREE_PID:-}" ]]; then
    TREE_PID="${STUTTER_TREE_PID}"
else
    sleep "$((SOAK_SECONDS + 30))" &
    TREE_PID="$!"
    TARGET_PID_CREATED=1
fi

cleanup() {
    if [[ "${TARGET_PID_CREATED}" == "1" ]]; then
        kill "${TREE_PID}" >/dev/null 2>&1 || true
        wait "${TREE_PID}" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

"${STUTTER}" autotune \
    --mode observe \
    --tree-pid "${TREE_PID}" \
    --duration-seconds "${SOAK_SECONDS}" \
    --summary-ms "${SUMMARY_MS}" \
    --preset "${PRESET}" \
    --decision-log "${DECISION_LOG}" \
    >"${STDOUT_LOG}" \
    2>"${STDERR_LOG}"

test -s "${DECISION_LOG}"
grep -F '"mode":"observe"' "${DECISION_LOG}" >/dev/null
grep -F '"decision":"noop"' "${DECISION_LOG}" >/dev/null

echo "autotune observe smoke passed"
echo "decision_log=${DECISION_LOG}"
echo "stdout_log=${STDOUT_LOG}"
echo "stderr_log=${STDERR_LOG}"
