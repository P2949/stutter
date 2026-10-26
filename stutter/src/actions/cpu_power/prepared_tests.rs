use super::*;

fn epp_only(root: &Path) -> CpuPowerAction {
    CpuPowerAction {
        sysfs_root: root.to_path_buf(),
        cpus: vec![0],
        scaling_governor: None,
        energy_performance_preference: Some("performance".to_owned()),
    }
}

fn describe_policy(cpufreq: &Path, related: &str, driver: &str, available: &str) {
    fs::write(cpufreq.join("related_cpus"), format!("{related}\n")).unwrap();
    fs::write(cpufreq.join("scaling_driver"), format!("{driver}\n")).unwrap();
    fs::write(
        cpufreq.join("energy_performance_available_preferences"),
        format!("{available}\n"),
    )
    .unwrap();
}

fn setup(name: &str) -> (PathBuf, PathBuf) {
    let root = temp_sysfs_root(name);
    let cpufreq = write_cpu_cpufreq(&root, 0, "powersave", "balance_performance");
    describe_policy(
        &cpufreq,
        "0",
        "intel_pstate",
        "default performance balance_performance balance_power power",
    );
    (root, cpufreq)
}

#[test]
fn the_hook_sees_the_original_value_with_its_cpufreq_identity_before_any_write() {
    let (root, cpufreq) = setup("prepared-hook");
    let epp = cpufreq.join("energy_performance_preference");
    let mut seen = None;
    let err = epp_only(&root)
        .apply_prepared_with_policy(&policy_for(&[0]), &mut |token| {
            // Nothing has been written when the journal sees the token.
            assert_eq!(
                fs::read_to_string(&epp).unwrap().trim(),
                "balance_performance"
            );
            seen = Some(token.clone());
            anyhow::bail!("journal refused")
        })
        .unwrap_err();
    assert!(format!("{:#}", err.source).contains("journal refused"));
    assert_eq!(
        fs::read_to_string(&epp).unwrap().trim(),
        "balance_performance"
    );
    let token = seen.expect("hook invoked");
    let records = token.as_cpu_power_restore().expect("cpu power token");
    assert_eq!(records.len(), 1);
    let identity = records[0].identity.as_ref().expect("identity recorded");
    assert_eq!(identity.attribute, "energy_performance_preference");
    assert_eq!(identity.related_cpus, vec![0]);
    assert_eq!(identity.driver.as_deref(), Some("intel_pstate"));
    assert_eq!(records[0].original_value, "balance_performance");
}

#[test]
fn a_value_that_drifted_after_preparation_aborts_the_apply_without_writing() {
    let (root, cpufreq) = setup("prepared-drift");
    let epp = cpufreq.join("energy_performance_preference");
    let epp_for_hook = epp.clone();
    let err = epp_only(&root)
        .apply_prepared_with_policy(&policy_for(&[0]), &mut |_| {
            fs::write(&epp_for_hook, "power\n").unwrap();
            Ok(())
        })
        .unwrap_err();
    assert!(format!("{:#}", err.source).contains("drifted"));
    assert_eq!(fs::read_to_string(&epp).unwrap().trim(), "power");
}

#[test]
fn a_prepared_apply_writes_verifies_and_returns_the_same_token() {
    let (root, cpufreq) = setup("prepared-apply");
    let mut journaled = None;
    let token = epp_only(&root)
        .apply_prepared_with_policy(&policy_for(&[0]), &mut |token| {
            journaled = Some(token.clone());
            Ok(())
        })
        .unwrap();
    assert_eq!(Some(token.clone()), journaled);
    assert_eq!(
        fs::read_to_string(cpufreq.join("energy_performance_preference"))
            .unwrap()
            .trim(),
        "performance"
    );
    rollback_verified(token.as_cpu_power_restore().unwrap(), true).unwrap();
    assert_eq!(
        fs::read_to_string(cpufreq.join("energy_performance_preference"))
            .unwrap()
            .trim(),
        "balance_performance"
    );
}

fn restore_after(change: impl FnOnce(&Path)) -> (usize, usize, String) {
    let (root, cpufreq) = setup("restore-identity");
    let token = epp_only(&root)
        .apply_prepared_with_policy(&policy_for(&[0]), &mut |_| Ok(()))
        .unwrap();
    change(&cpufreq);
    let outcome = rollback_verified(token.as_cpu_power_restore().unwrap(), true).unwrap();
    (
        outcome.restored,
        outcome.skipped_identity_mismatch,
        fs::read_to_string(cpufreq.join("energy_performance_preference"))
            .unwrap()
            .trim()
            .to_owned(),
    )
}

#[test]
fn a_restore_never_replays_onto_a_different_policy_driver_or_illegal_value() {
    // Control: nothing changed, the value is restored.
    assert_eq!(
        restore_after(|_| {}),
        (1, 0, "balance_performance".to_owned())
    );
    // The related CPU set changed (another policy now owns the path).
    let (restored, skipped, value) = restore_after(|dir| {
        fs::write(dir.join("related_cpus"), "0 1\n").unwrap();
    });
    assert_eq!((restored, skipped, value.as_str()), (0, 1, "performance"));
    // A different driver.
    let (restored, skipped, _) = restore_after(|dir| {
        fs::write(dir.join("scaling_driver"), "acpi-cpufreq\n").unwrap();
    });
    assert_eq!((restored, skipped), (0, 1));
    // The original value is no longer a legal value of the attribute.
    let (restored, skipped, _) = restore_after(|dir| {
        fs::write(
            dir.join("energy_performance_available_preferences"),
            "default performance\n",
        )
        .unwrap();
    });
    assert_eq!((restored, skipped), (0, 1));
}

#[test]
fn a_record_without_a_cpufreq_identity_is_never_replayed_automatically() {
    let (_, cpufreq) = setup("restore-legacy");
    let record = CpuPowerRestoreRecord {
        path: cpufreq.join("energy_performance_preference"),
        original_value: "power".to_owned(),
        identity: None,
    };
    let outcome = rollback_verified(&[record.clone()], true).unwrap();
    assert_eq!(
        (outcome.restored, outcome.skipped_identity_mismatch),
        (0, 1)
    );
    assert_eq!(
        fs::read_to_string(&record.path).unwrap().trim(),
        "balance_performance"
    );
    // The manual restore path keeps replaying legacy records.
    assert_eq!(rollback_verified(&[record], false).unwrap().restored, 1);
}
