#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly-2026-06-06}"

run_test() {
  local label="$1"
  local filter="$2"

  echo "--- SAFETY: ${label} (${filter}) ---"
  RUSTUP_TOOLCHAIN="$TOOLCHAIN" cargo test -p stutter "$filter" --lib
}

run_test "planner golden fixtures" "planner_golden_cases"
run_test "high-risk apply disabled" "apply_high_risk_is_disabled_even_with_explicit_high_risk_unlock"
run_test "manual-only high-risk planner gate" "manual_only_high_risk_candidate_is_never_selected_for_apply_modes"
run_test "medium-risk explicit unlock" "apply_medium_risk_allows_medium_risk_only_when_explicit"
run_test "medium-risk privileged service required" "runtime_executor_requires_privileged_service_for_medium_risk_apply"
run_test "protected task classes excluded" "protected_task_classes_are_never_selected"
run_test "explicit protected tasks excluded" "explicit_protected_tasks_are_never_selected"
run_test "protected IRQ CPU excluded" "irq_provider_rejects_when_only_candidate_cpu_is_protected"
run_test "cgroup provider excludes protected tasks" "cgroup_provider_moves_only_allowed_non_protected_target_tasks"
run_test "privilege boundary rejects remote privileged operation" "remote_tcp_cannot_request_privileged_operation_even_with_apply_auth"
run_test "privileged service rejects stale plans" "privileged_action_service_rejects_stale_candidate_plan_before_execution"
run_test "planner no-op detection" "no_effective_change_candidate_is_denied_before_dry_run"
run_test "CPU affinity no-op detection" "cpu_affinity_no_effective_change_is_denied_before_dry_run"
run_test "rollback verification" "rollback_verification"
run_test "scenario soak safety fixtures" "scenario_driven_soak_fixtures_preserve_safety_invariants"
run_test "fake low-risk soak rollback tracking" "fake_low_risk_soak_tracks_actions_and_rollbacks"

echo "PASS autotune safety matrix"
