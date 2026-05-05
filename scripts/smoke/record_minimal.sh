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

export RUST_LOG="${RUST_LOG:-info}"

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
    echo "--- output.log ---"
    cat "${OUT_BASE}/output.log"
    exit $STATUS
fi

# Verification of JSON artifacts
echo "Verifying recording artifacts..."

# Locate actual recording directory:
# a. first check $RECORD_DIR/session.json
# b. otherwise search one level under $RECORD_DIR for the newest directory containing session.json
ACTUAL_DIR=""
if [ -f "${RECORD_DIR}/session.json" ]; then
    ACTUAL_DIR="${RECORD_DIR}"
else
    # find one level deeper for the newest directory with session.json
    ACTUAL_DIR=$(find "${RECORD_DIR}" -maxdepth 2 -name session.json -printf "%T@ %h\n" | sort -rn | head -n1 | cut -d' ' -f2- || true)
fi

if [ -z "${ACTUAL_DIR}" ] || [ ! -d "${ACTUAL_DIR}" ]; then
    echo "FAIL: Could not locate recording directory containing session.json under ${RECORD_DIR}"
    ls -R "${RECORD_DIR}" || true
    exit 1
fi

echo "Validating artifacts in: ${ACTUAL_DIR}"

# metadata.json and session.json are REQUIRED
for req in metadata.json session.json; do
    if [ ! -f "${ACTUAL_DIR}/${req}" ]; then
        echo "FAIL: Required artifact ${req} is missing!"
        exit 1
    fi
    echo "Validating ${req}..."
    python3 -c "import json; json.load(open('${ACTUAL_DIR}/${req}'))"
done

# Optional NDJSON files (validate only if present)
for opt in interval.json tree_events.json scx_events.json irq_events.json gpu_samples.json frame_events.json block_io_events.json; do
    if [ -f "${ACTUAL_DIR}/${opt}" ]; then
        echo "Validating ${opt} (NDJSON)..."
        # NDJSON validation: each line must be valid JSON
        python3 -c "import json; [json.loads(line) for line in open('${ACTUAL_DIR}/${opt}') if line.strip()]"
    fi
done

echo "PASS: Minimal record smoke test successful."
