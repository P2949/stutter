#!/usr/bin/env bash
set -euo pipefail

: "${RUSTUP_TOOLCHAIN:=nightly-2026-06-06}"

run() {
  echo
  echo "+ $*"
  "$@"
}

run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" \
  cargo test -p stutter action_error_framework_path_excludes_rollback_directory

run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" \
  cargo test -p stutter action_error_framework_path_does_not_overmatch_rollback_prefix

run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" \
  cargo test -p stutter action_boundary_error_round_trips_through_action_error_serde

run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" \
  cargo test -p stutter raw_string_action_and_experiment_ids_are_tracked_until_migrated

run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" \
  cargo test -p stutter raw_string_id_allowlist_entries_have_reasons_and_exit_criteria

run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" \
  cargo test -p stutter architecture_tests::action_errors

run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" \
  cargo test -p stutter architecture_tests::string_id_validation

run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" \
  cargo test -p stutter actions::error
