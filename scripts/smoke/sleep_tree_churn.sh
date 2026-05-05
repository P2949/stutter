#!/usr/bin/env bash
set -euo pipefail

# Navigate to repo root from scripts/smoke
cd "$(dirname "$0")/../.."

# Sourcing lib.sh for helper functions (assumed to be in scripts/smoke/lib.sh)
source scripts/smoke/lib.sh

# Create output directory
OUT_DIR=$(make_smoke_dir "sleep-tree-churn")

# Save basic metadata
write_basic_metadata "${OUT_DIR}"

# Start a background process that churns children for ~10 seconds
echo "Starting background tree churn process..."
bash -c '
    end=$(( $(date +%s) + 10 ))
    while [ $(date +%s) -lt $end ]; do
        # Spawn short-lived children
        /usr/bin/true &
        sleep 0.1
    done
' &
ROOT_PID=$!

# Cleanup background process on exit
cleanup() {
    echo "Stopping background churner (PID $ROOT_PID)..."
    kill "$ROOT_PID" 2>/dev/null || true
    wait "$ROOT_PID" 2>/dev/null || true
}
trap cleanup EXIT

export RUST_LOG="${RUST_LOG:-info}"

# Verify stutter binary existence
STUTTER_BIN="$(pwd)/target/debug/stutter"
if [ ! -f "$STUTTER_BIN" ]; then
    echo "stutter binary not found, building..."
    cargo build
fi

# Define the monitor command
# Using exact CLI flags: monitor, --tree-pid, --no-record
CMD=("$STUTTER_BIN" monitor --tree-pid "$ROOT_PID" --no-record)
echo "${CMD[@]}" > "${OUT_DIR}/command.txt"

echo "Running stutter monitor against tree PID $ROOT_PID for 7 seconds..."
# run_or_skip_live captures output to output.log in the current directory.
# We pushd to the output directory so the log is saved there.
pushd "${OUT_DIR}" > /dev/null
STATUS=0
run_or_skip_live timeout 7s "${CMD[@]}" || STATUS=$?
popd > /dev/null

if [ $STATUS -eq 77 ]; then
    echo "SKIP: live eBPF smoke requires root/capabilities"
    exit 0
elif [ $STATUS -eq 124 ]; then
    echo "timeout expected (status 124); continuing to validation..."
elif [ $STATUS -ne 0 ]; then
    echo "FAIL: stutter monitor failed with status $STATUS"
    echo "--- output.log ---"
    cat "${OUT_DIR}/output.log"
    exit $STATUS
fi

# Validate output markers
LOG="${OUT_DIR}/output.log"
# Requirements: output.log must contain at least one of: tree_target_added, tree_target_removed, summary, session
if grep -qiE "tree_target_added|tree_target_removed|target_added|target_removed|summary|session" "$LOG"; then
    echo "PASS: Smoke test successful. Found expected markers in $LOG."
else
    echo "FAIL: Expected markers not found in $LOG."
    echo "--- Log Content ---"
    cat "$LOG"
    exit 1
fi
