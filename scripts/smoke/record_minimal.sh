#!/usr/bin/env bash
set -euo pipefail

# Navigate to repo root from scripts/smoke
cd "$(dirname "$0")/../.."

# Source the helper library
source scripts/smoke/lib.sh

# Create output base directory
OUT_BASE=$(make_smoke_dir "record-minimal")

# Save basic metadata
write_basic_metadata "${OUT_BASE}"

# Start a small background churn process
echo "Starting background churn process..."
bash -c 'while true; do /usr/bin/true; sleep 0.1; done' &
CHURN_PID=$!

# Cleanup on exit
cleanup() {
    echo "Stopping background churner (PID $CHURN_PID)..."
    kill "$CHURN_PID" 2>/dev/null || true
    wait "$CHURN_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Ensure stutter is built
STUTTER_BIN="$(pwd)/target/debug/stutter"
if [ ! -f "$STUTTER_BIN" ]; then
    echo "stutter binary not found, building..."
    cargo build
fi

# Define the record command
# Exact CLI names: record, --tree-pid, --duration, --out-dir
RECORD_DIR="${OUT_BASE}/recording"
CMD=("$STUTTER_BIN" record --tree-pid "$CHURN_PID" --duration 5 --out-dir "$RECORD_DIR")
echo "${CMD[@]}" > "${OUT_BASE}/command.txt"

echo "Running stutter record for 5 seconds..."
# run_or_skip_live captures to output.log in the current directory.
# We pushd to OUT_BASE to keep artifacts together.
pushd "${OUT_BASE}" > /dev/null
STATUS=0
run_or_skip_live "${CMD[@]}" || STATUS=$?
popd > /dev/null

# Handle skip condition
if [ $STATUS -eq 77 ]; then
    echo "SKIP: live eBPF smoke requires root/capabilities"
    exit 0
fi

if [ $STATUS -ne 0 ]; then
    echo "FAIL: stutter record exited with status $STATUS"
    exit $STATUS
fi

# Verification of JSON artifacts
echo "Verifying recording artifacts in $RECORD_DIR..."

# metadata.json and session.json are REQUIRED
for req in metadata.json session.json; do
    if [ ! -f "${RECORD_DIR}/${req}" ]; then
        echo "FAIL: Required artifact ${req} is missing!"
        exit 1
    fi
    echo "Validating ${req}..."
    python3 -c "import json; json.load(open('${RECORD_DIR}/${req}'))"
done

# interval.json and tree_events.json are OPTIONAL (validate if present)
# They are NDJSON formatted.
for opt in interval.json tree_events.json; do
    if [ -f "${RECORD_DIR}/${opt}" ]; then
        echo "Validating ${opt} (NDJSON)..."
        python3 -c "import json; [json.loads(line) for line in open('${RECORD_DIR}/${opt}')]"
    else
        echo "Note: Optional artifact ${opt} is absent."
    fi
done

echo "PASS: Minimal record smoke test successful."
