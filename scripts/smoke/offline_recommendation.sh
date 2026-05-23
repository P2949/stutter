#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

baseline="$tmp/baseline"
tune="$tmp/tune"
mkdir -p "$baseline" "$tune"

cat >"$baseline/session.json" <<'JSON'
{
  "schema_version": 18,
  "run_name": "baseline",
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
  "interval_record_count": 1,
  "frame_event_count": 1,
  "tasks": [],
  "top_spikes": []
}
JSON

cat >"$baseline/interval.json" <<'JSON'
{"elapsed_ms":1000,"task":1234,"active":true,"class":"Game","comm":"Game.exe","process_pid":1234,"process_comm":"Game.exe","samples":100,"stored_samples":100,"truncated_samples":0,"min_ns":100000,"avg_ns":1000000,"p95_ns":6000000,"p99_ns":8000000,"max_ns":9000000,"over_1ms":10,"over_2ms":10,"over_5ms":10,"busiest_cpu":0,"busiest_cpu_samples":100,"worst_cpu":0,"worst_cpu_max_ns":9000000,"spikiest_cpu":0,"spikiest_cpu_spikes":10}
JSON

cat >"$baseline/frame_correlation.json" <<'JSON'
{"elapsed_ms":1000,"frametime_ms":12.0}
JSON

cat >"$tune/tuning_summary.json" <<'JSON'
{
  "schema_version": 1,
  "tree_pid": 1234,
  "profiles_path": "profiles.toml",
  "runs": 5,
  "epoch_seconds": 30,
  "warmup_seconds": 5,
  "restore_policy": "restore-after-each-candidate",
  "best_profile": "game-main-on-2-5",
  "candidate_order": [{"iteration": 1, "profiles": ["game-main-on-2-5", "baseline-online"]}],
  "profile_stats": [
    {
      "profile": "game-main-on-2-5",
      "valid_runs": 5,
      "invalid_runs": 0,
      "median_diagnostic_score_total": 900,
      "iqr_diagnostic_score_total": 10,
      "worst_diagnostic_score_total": 950,
      "median_over_5ms": 7,
      "iqr_over_5ms": 1,
      "median_frame_p99_us": 12000,
      "iqr_frame_p99_us": 0
    },
    {
      "profile": "baseline-online",
      "valid_runs": 5,
      "invalid_runs": 0,
      "median_diagnostic_score_total": 1210,
      "iqr_diagnostic_score_total": 0,
      "worst_diagnostic_score_total": 1210,
      "median_over_5ms": 10,
      "iqr_over_5ms": 0,
      "median_frame_p99_us": 12000,
      "iqr_frame_p99_us": 0
    }
  ],
  "ranking_confidence": "High",
  "ranking_notes": [],
  "comparability_warnings": [],
  "candidates": [
    {
      "profile": "game-main-on-2-5",
      "iteration": 1,
      "run_dir": "candidate",
      "applied_tasks": 4,
      "warmup_seconds": 5,
      "measure_seconds": 30,
      "interval_count": 1,
      "samples": 100,
      "scored_samples": 100,
      "diagnostic_score_total": 900,
      "over_1ms": 8,
      "over_2ms": 8,
      "over_5ms": 7,
      "max_latency_ns": 8000000,
      "frame_count": 1,
      "frame_max_ms": 12.0,
      "frame_p99_ms": 12.0,
      "frame_over_16ms": 0,
      "frame_over_33ms": 0,
      "frame_over_50ms": 0,
      "coverage": {
        "unique_tracked_tasks": 1,
        "unique_scored_tasks": 1,
        "active_target_min": 1,
        "active_target_max": 1,
        "removed_task_count": 0,
        "drop_counter_total": 0,
        "scored_identity_counts": [
          {
            "identity": {
              "class": "Game",
              "process_comm": "Game.exe",
              "comm": "Game.exe",
              "process_starttime_ticks": null,
              "task_starttime_ticks": null,
              "exe_dev": null,
              "exe_ino": null
            },
            "count": 1
          }
        ]
      },
      "valid": true
    }
  ]
}
JSON

md_out="$tmp/recommend.md"
json_out="$tmp/recommend.json"

RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly}" cargo run -p stutter --quiet -- recommend --baseline "$baseline" --tune "$tune" >"$md_out"
grep -q "Verdict" "$md_out"
grep -q "Best profile" "$md_out"

RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly}" cargo run -p stutter --quiet -- recommend --baseline "$baseline" --tune "$tune" --json >"$json_out"
python3 - "$json_out" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    data = json.load(f)
assert data["best_profile"] == "game-main-on-2-5"
assert data["verdict"] in {"Recommended", "NeedsRetest", "NoRecommendation"}
PY

echo "PASS offline_recommendation"
