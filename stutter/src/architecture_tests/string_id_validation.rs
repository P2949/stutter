use std::collections::BTreeMap;

#[test]
fn stutter_core_string_ids_have_fallible_non_empty_constructors() {
    let root = crate::architecture_tests::workspace_root();
    let source = std::fs::read_to_string(root.join("stutter-core/src/ids.rs"))
        .expect("read stutter-core ids.rs");

    assert!(
        source.contains("pub struct EmptyStringIdError"),
        "stutter-core string IDs should expose EmptyStringIdError"
    );
    assert!(
        source.contains(
            "pub fn try_new(value: impl Into<String>) -> Result<Self, EmptyStringIdError>"
        ),
        "string_id! macro should provide try_new()"
    );
    assert!(
        source.contains("pub fn validate_non_empty(&self) -> Result<(), EmptyStringIdError>"),
        "string_id! macro should provide validate_non_empty() for deserialized values"
    );
}

#[test]
fn stutter_report_load_validates_deserialized_run_id() {
    let root = crate::architecture_tests::workspace_root();
    let load_source = std::fs::read_to_string(root.join("stutter-report/src/load.rs"))
        .expect("read stutter-report load.rs");
    let model_source = std::fs::read_to_string(root.join("stutter-report/src/model/root.rs"))
        .expect("read stutter-report model root");

    assert!(
        model_source.contains("validate_identity_strings"),
        "ReportModel should expose identity validation for serde-loaded IDs"
    );
    assert!(
        load_source.contains("validate_identity_strings()"),
        "stutter-report loader should validate deserialized string IDs"
    );
}

#[test]
fn autotune_experiment_id_uses_shared_core_id_type() {
    let root = crate::architecture_tests::workspace_root();
    let source = std::fs::read_to_string(root.join("stutter/src/autotune/experiment.rs"))
        .expect("read autotune experiment.rs");

    assert!(
        source.contains("pub use stutter_core::ids::ExperimentId;"),
        "autotune should use stutter_core::ids::ExperimentId instead of a duplicate local string ID"
    );
    assert!(
        !source.contains("pub struct ExperimentId(pub String)"),
        "autotune must not reintroduce a duplicate local ExperimentId"
    );
}

#[test]
fn daemon_policy_and_ipc_boundaries_validate_action_ids() {
    let root = crate::architecture_tests::workspace_root();

    // Policy Check
    let evaluate_source =
        std::fs::read_to_string(root.join("stutter/src/daemon/policy/evaluate/reason.rs"))
            .expect("read evaluate reason.rs");
    assert!(
        evaluate_source.contains("validate_identity_strings()"),
        "daemon policy evaluation must validate ActionDescriptor ActionId"
    );

    // Privileged Worker IPC Check
    let priv_model_source =
        std::fs::read_to_string(root.join("stutter/src/daemon/privilege/model.rs"))
            .expect("read priv model.rs");
    assert!(
        priv_model_source.contains("validate_identity_strings()"),
        "privileged worker candidate plan deserialization must validate ActionId"
    );

    // Candidate Memory Check
    let candidate_memory_model =
        std::fs::read_to_string(root.join("stutter/src/autotune/candidate_memory/model.rs"))
            .expect("read candidate memory model.rs");
    assert!(
        candidate_memory_model.contains("validate_identity_strings()"),
        "CandidateMemory must validate ActionId during deserialization"
    );

    // Controller Journal Check
    let journal_model =
        std::fs::read_to_string(root.join("stutter/src/autotune/controller_journal/model.rs"))
            .expect("read journal model.rs");
    assert!(
        journal_model.contains("validate_identity_strings()"),
        "read_controller_journal must validate ActionId and ExperimentId during deserialization"
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RawStringIdAllowance {
    path: &'static str,
    needle: &'static str,
    expected_count: usize,
    reason: &'static str,
    exit_criteria: &'static str,
}

const RAW_STRING_ID_ALLOWLIST: &[RawStringIdAllowance] = &[
    RawStringIdAllowance {
        path: "stutter/src/audit.rs",
        needle: "pub action_id: Option<String>,",
        expected_count: 1,
        reason: "audit JSON compatibility remains string-shaped while audit records migrate separately from autotune/daemon state IDs",
        exit_criteria: "replace AuditEvent.action_id with ActionId or validate_non_empty() when reading audit events",
    },
    RawStringIdAllowance {
        path: "stutter/src/commands/daemon/status.rs",
        needle: "pub action_id: Option<String>,",
        expected_count: 1,
        reason: "daemon status recent-decision JSON is a display DTO while persisted history and daemon state IDs migrate to typed IDs",
        exit_criteria: "replace DaemonRecentDecision.action_id with ActionId or validate_non_empty() when building status output",
    },
    RawStringIdAllowance {
        path: "stutter/src/commands/daemon/watch.rs",
        needle: "pub active_action_id: Option<String>,",
        expected_count: 1,
        reason: "daemon watch signatures are display/cache DTOs while active daemon state IDs are typed",
        exit_criteria: "replace DaemonWatchSignature.active_action_id with ActionId or validate_non_empty() when building watch signatures",
    },
    RawStringIdAllowance {
        path: "stutter/src/commands/daemon/watch.rs",
        needle: "pub rollback_action_id: Option<String>,",
        expected_count: 1,
        reason: "daemon watch signatures are display/cache DTOs while rollback daemon state IDs are typed",
        exit_criteria: "replace DaemonWatchSignature.rollback_action_id with ActionId or validate_non_empty() when building watch signatures",
    },
];

fn is_raw_string_id_scan_excluded_path(path: &str) -> bool {
    path.contains("/tests/")
        || path.ends_with("/tests.rs")
        || path.ends_with("_tests.rs")
        || path.contains("/fixtures/")
        || path.contains("/test_fixture")
        || path.contains("/architecture_tests/")
}

fn raw_string_id_needle(line: &str) -> Option<&str> {
    let trimmed = line.trim();

    if trimmed.contains("action_id: String")
        || trimmed.contains("action_id: Option<String>")
        || trimmed.contains("experiment_id: String")
        || trimmed.contains("experiment_id: Option<String>")
    {
        return Some(trimmed);
    }

    None
}

fn raw_string_id_allowance_for(path: &str, needle: &str) -> Option<&'static RawStringIdAllowance> {
    RAW_STRING_ID_ALLOWLIST
        .iter()
        .find(|allowance| allowance.path == path && allowance.needle == needle)
}

fn raw_string_id_allowlist_map() -> BTreeMap<(String, String), usize> {
    RAW_STRING_ID_ALLOWLIST
        .iter()
        .map(|allowance| {
            (
                (allowance.path.to_owned(), allowance.needle.to_owned()),
                allowance.expected_count,
            )
        })
        .collect()
}

#[test]
fn raw_string_id_scanner_preserves_exact_declaration_needles() {
    assert_eq!(
        raw_string_id_needle("    pub action_id: String,"),
        Some("pub action_id: String,")
    );
    assert_eq!(
        raw_string_id_needle("    pub(crate) experiment_id: String,"),
        Some("pub(crate) experiment_id: String,")
    );
    assert_eq!(
        raw_string_id_needle("        action_id: String,"),
        Some("action_id: String,")
    );
    assert_eq!(
        raw_string_id_needle("    pub action_id: Option<String>,"),
        Some("pub action_id: Option<String>,")
    );
}

#[test]
fn raw_string_id_scanner_ignores_non_identity_lines() {
    assert_eq!(raw_string_id_needle("pub action: String,"), None);
    assert_eq!(raw_string_id_needle("pub experiment: String,"), None);
    assert_eq!(raw_string_id_needle("pub action_id: ActionId,"), None);
    assert_eq!(
        raw_string_id_needle("pub experiment_id: ExperimentId,"),
        None
    );
}

#[test]
fn raw_string_id_path_filter_excludes_tests_fixtures_and_architecture_tests() {
    for path in [
        "stutter/src/foo/tests.rs",
        "stutter/src/foo/tests/bar.rs",
        "stutter/src/foo/foo_tests.rs",
        "stutter/src/tests/fixtures/example.rs",
        "stutter/src/test_fixture_builder/model.rs",
        "stutter/src/architecture_tests/string_id_validation.rs",
    ] {
        assert!(
            is_raw_string_id_scan_excluded_path(path),
            "{path} should be excluded from raw string ID scanning"
        );
    }
}

#[test]
fn raw_string_id_allowance_lookup_requires_exact_path_and_needle() {
    assert!(
        raw_string_id_allowance_for("stutter/src/daemon/state.rs", "pub action_id: String,")
            .is_none(),
        "migrated raw ID groups should not remain allowlisted"
    );
}

#[test]
fn raw_string_action_and_experiment_ids_are_tracked_until_migrated() {
    let root = crate::architecture_tests::workspace_root();
    let stutter_src = root.join("stutter/src");

    let mut actual_counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut untracked = Vec::new();

    for file in crate::architecture_tests::scanners::rust_files_under(&stutter_src) {
        let relative = crate::architecture_tests::relative_to_workspace_root(&file);

        if is_raw_string_id_scan_excluded_path(&relative) {
            continue;
        }

        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));

        for (line_index, line) in source.lines().enumerate() {
            let Some(needle) = raw_string_id_needle(line) else {
                continue;
            };

            let key = (relative.clone(), needle.to_owned());
            *actual_counts.entry(key).or_insert(0) += 1;

            if raw_string_id_allowance_for(&relative, needle).is_none() {
                untracked.push(format!(
                    "{}:{}: raw `{}` should use ActionId/ExperimentId or be added to RAW_STRING_ID_ALLOWLIST with reason and exit criteria",
                    relative,
                    line_index + 1,
                    needle
                ));
            }
        }
    }

    let expected_counts = raw_string_id_allowlist_map();

    let mut count_mismatches = Vec::new();
    for allowance in RAW_STRING_ID_ALLOWLIST {
        let actual = actual_counts
            .get(&(allowance.path.to_owned(), allowance.needle.to_owned()))
            .copied()
            .unwrap_or(0);

        if actual != allowance.expected_count {
            count_mismatches.push(format!(
                "{}: expected {} occurrence(s) of `{}`, found {}",
                allowance.path, allowance.expected_count, allowance.needle, actual
            ));
        }
    }

    let mut stale_or_unexpected = Vec::new();
    for ((path, needle), actual) in &actual_counts {
        if !expected_counts.contains_key(&(path.clone(), needle.clone())) {
            stale_or_unexpected.push(format!(
                "{path}: found unallowlisted `{needle}` {actual} time(s)"
            ));
        }
    }

    assert!(
        untracked.is_empty() && count_mismatches.is_empty() && stale_or_unexpected.is_empty(),
        "raw action_id/experiment_id String migration is not fully tracked\n\nuntracked raw IDs:\n{}\n\ncount mismatches:\n{}\n\nunexpected groups:\n{}",
        untracked.join("\n"),
        count_mismatches.join("\n"),
        stale_or_unexpected.join("\n")
    );
}

#[test]
fn raw_string_id_allowlist_entries_have_reasons_and_exit_criteria() {
    for allowance in RAW_STRING_ID_ALLOWLIST {
        assert!(
            !allowance.reason.trim().is_empty(),
            "raw string ID allowance for {} / {} must explain why it remains",
            allowance.path,
            allowance.needle
        );
        assert!(
            !allowance.exit_criteria.trim().is_empty(),
            "raw string ID allowance for {} / {} must include concrete exit criteria",
            allowance.path,
            allowance.needle
        );
        assert!(
            allowance.exit_criteria.contains("replace")
                || allowance.exit_criteria.contains("validate_non_empty"),
            "raw string ID allowance for {} / {} must describe replacement or validation exit criteria",
            allowance.path,
            allowance.needle
        );
        assert!(
            allowance.expected_count > 0,
            "raw string ID allowance for {} / {} must track at least one current occurrence",
            allowance.path,
            allowance.needle
        );
    }
}
