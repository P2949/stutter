#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

run="$tmp/run"
mkdir -p "$run"

cat >"$run/session.json" <<'JSON'
{
  "schema_version": 18,
  "run_name": "advisor-offline",
  "started_at": {"unix_seconds": 0, "unix_nanos": 0, "system_time_debug": "test"},
  "ended_at": {"unix_seconds": 1, "unix_nanos": 0, "system_time_debug": "test"},
  "monotonic_start_ns": null,
  "monotonic_end_ns": null,
  "duration_ms": 1000,
  "stop_reason": "test",
  "config": {
    "manual_pids": [1234],
    "tree_roots": [],
    "summary_period_ms": 1000,
    "spike_threshold_ns": 1000000,
    "verbose": false
  },
  "metadata": {
    "kernel_osrelease": null,
    "kernel_version": null,
    "cpu_online": null,
    "cpu_possible": null,
    "cpu_topology": [],
    "scx_state": null,
    "scx_ops": null,
    "scx_enable_seq": null
  },
  "target_pids_max": 1024,
  "active_target_pids_count": 1,
  "active_expanded_tasks": [1234],
  "interval_record_count": 0,
  "spike_events_retained_count": 0,
  "frame_event_count": 0,
  "tasks": [],
  "top_spikes": []
}
JSON

md_out="$tmp/advisor.md"
json_out="$tmp/advisor.json"

RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly-2026-06-06}" cargo run -p stutter --quiet -- advisor --run "$run" >"$md_out"
grep -q "Verdict" "$md_out"

RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly-2026-06-06}" cargo run -p stutter --quiet -- advisor --run "$run" --json >"$json_out"
python3 - "$json_out" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    data = json.load(f)
assert data["schema_version"] == 1
assert "verdict" in data
assert isinstance(data["recommendations"], list)
PY

echo "PASS advisor_offline"
