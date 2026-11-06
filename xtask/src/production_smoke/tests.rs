use super::*;

#[test]
fn systemctl_show_output_is_parsed_into_properties() {
    let text = "ActiveState=active\nSubState=running\nExecStart={ path=/nix/store/abc-stutter ; argv[]=/nix/store/abc-stutter autotune --mode x ; ignore_errors=no }\nUser=\n";
    let properties = parse_show(text);
    assert_eq!(properties["ActiveState"], "active");
    assert_eq!(properties["User"], "");
    assert_eq!(
        exec_start_path(&properties["ExecStart"]).as_deref(),
        Some("/nix/store/abc-stutter")
    );
    assert_eq!(exec_start_path("{ ignore_errors=no }"), None);
}

#[test]
fn process_status_fields_are_read_as_written() {
    let status =
        "Name:\tstutter\nUid:\t1000\t1000\t1000\t1000\nCapEff:\t000000c001000004\nNoNewPrivs:\t1\n";
    assert_eq!(
        status_field(status, "CapEff").as_deref(),
        Some(CONTROLLER_CAPS)
    );
    assert_eq!(status_field(status, "NoNewPrivs").as_deref(), Some("1"));
    assert_eq!(status_field(status, "Missing"), None);
}

#[test]
fn only_unrestored_tokens_are_outstanding() {
    let ledger = r#"[{"restored":true},{"restored":true},{"restored":false},{}]"#;
    assert_eq!(ledger_counts(ledger).unwrap(), (2, 4));
    assert_eq!(ledger_counts("[]").unwrap(), (0, 0));
    assert!(ledger_counts("not json").is_err());
}

#[test]
fn the_pinned_binary_account_and_capabilities_are_all_required() {
    let mut properties = BTreeMap::new();
    properties.insert("ActiveState".to_owned(), "active".to_owned());
    properties.insert("SubState".to_owned(), "running".to_owned());
    properties.insert("User".to_owned(), "someone".to_owned());
    properties.insert(
        "FragmentPath".to_owned(),
        "/run/systemd/transient/x.service".to_owned(),
    );
    properties.insert("MainPID".to_owned(), "0".to_owned());
    let checks = unit_checks(
        "controller",
        &properties,
        &"0".repeat(64),
        "root",
        "0",
        0,
        true,
    );
    let failed: Vec<_> = checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.name.as_str())
        .collect();
    // Transient units, the wrong account and a missing binary or process all fail.
    assert!(failed.contains(&"controller_declared_by_the_host"));
    assert!(failed.contains(&"controller_account"));
    assert!(failed.contains(&"controller_binary_is_immutable"));
    assert!(failed.contains(&"controller_process_identity"));
}

#[test]
fn an_experiment_counts_only_for_the_fixture_and_only_after_it_was_applied() {
    let record = |pid: u64, decision: &str| json!({"target": {"root_pid": pid}, "decision": {"decision": decision}});
    let mut outcome = Outcome::default();
    // A resolution without an apply, and another workload's apply, prove nothing.
    fold_outcome(
        &mut outcome,
        &[record(7, "candidate_kept"), record(9, "candidate_applied")],
        7,
    );
    assert_eq!(outcome, Outcome::default());
    fold_outcome(
        &mut outcome,
        &[
            record(7, "candidate_applied"),
            record(7, "candidate_reverted"),
        ],
        7,
    );
    assert!(outcome.applied);
    assert_eq!(outcome.resolution.as_deref(), Some("candidate_reverted"));
}

#[test]
fn a_protected_chain_must_be_root_owned_and_not_writable() {
    // The system root is, and a user's scratch directory is not.
    assert!(immutable_chain(Path::new("/nix/store")).is_ok());
    let scratch = std::env::temp_dir();
    assert!(immutable_chain(&scratch).is_err());
}

#[test]
fn new_records_returns_complete_lines_and_survives_rotation() {
    let dir = std::env::temp_dir().join(format!("smoke-records-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("history.jsonl");
    fs::write(&path, "{\"a\":1}\n{\"a\":2}\n{\"a\":").unwrap();
    let mut offset = 0;
    assert_eq!(new_records(&path, &mut offset).len(), 2);
    // The partial line is read once it is complete.
    fs::write(&path, "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n").unwrap();
    assert_eq!(new_records(&path, &mut offset).len(), 1);
    // A rotated (smaller) file starts again.
    fs::write(&path, "{\"b\":1}\n").unwrap();
    assert_eq!(new_records(&path, &mut offset).len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

mod matrix_helpers {
    use super::super::matrix::{cpu_percent, stale_temporaries};

    #[test]
    fn cpu_percent_is_ticks_over_the_clock_rate_and_the_window() {
        // 100 ticks at 100 Hz is one second of CPU; over a ten second window that is 10 percent.
        assert!((cpu_percent(100, 100, 10.0) - 10.0).abs() < 1e-9);
        assert_eq!(cpu_percent(0, 100, 30.0), 0.0);
    }

    #[test]
    fn only_temporaries_of_dead_processes_count_as_stale() {
        let dir = std::env::temp_dir().join(format!("smoke-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("candidate-memory.tmp-4294967290"), b"x").unwrap();
        std::fs::write(
            dir.join(format!("candidate-memory.tmp-{}", std::process::id())),
            b"x",
        )
        .unwrap();
        std::fs::write(dir.join("candidate-memory.json"), b"x").unwrap();
        assert_eq!(stale_temporaries(&dir), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
