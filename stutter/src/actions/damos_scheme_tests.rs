use std::fs;

use super::*;

fn admin(name: &str, state: &str, contexts: u32) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-damos-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let kd = root.join("kdamonds");
    let ctx = kd.join("0/contexts/0");
    for dir in [
        ctx.join("targets/0"),
        ctx.join("schemes/0/quotas/goals/0"),
        ctx.join("schemes/0/access_pattern/sz"),
    ] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(kd.join("nr_kdamonds"), "1").unwrap();
    fs::write(kd.join("0/state"), state).unwrap();
    fs::write(kd.join("0/contexts/nr_contexts"), contexts.to_string()).unwrap();
    root
}

fn request(action: DamosAction) -> DamosRequest {
    DamosRequest {
        task: 4242,
        start_ticks: 777,
        action,
        quota: DamosQuota {
            ms: 5,
            bytes: 1 << 20,
            reset_interval_ms: 1000,
        },
        acknowledge_irreversible: true,
        min_region_bytes: 4096,
        max_region_bytes: 1 << 30,
        goal: None,
    }
}

fn alive(task: u32) -> Option<u64> {
    (task == 4242).then_some(777)
}

fn read(root: &Path, file: &str) -> String {
    fs::read_to_string(root.join("kdamonds").join(file))
        .unwrap()
        .trim()
        .to_owned()
}

#[test]
fn a_monitoring_scheme_commits_reads_back_and_clears() {
    let root = admin("stat", "off", 0);
    let prepared = prepare(&root, &request(DamosAction::Stat), alive).unwrap();
    assert_eq!(read(&root, "0/state"), "off", "prepare writes nothing");
    let committed = prepared.commit().unwrap();
    assert_eq!(read(&root, "0/state"), "on");
    assert_eq!(read(&root, "0/contexts/0/targets/0/pid_target"), "4242");
    assert_eq!(
        committed.restore(),
        DamosRestore::Cleared {
            irreversible_effect_possible: false
        }
    );
    assert_eq!(read(&root, "0/state"), "off");
    assert_eq!(read(&root, "0/contexts/nr_contexts"), "0");
}

#[test]
fn reclaim_is_classed_irreversible_and_needs_acknowledgement_and_a_bounded_quota() {
    let root = admin("pageout", "off", 0);
    let mut unacknowledged = request(DamosAction::Pageout);
    unacknowledged.acknowledge_irreversible = false;
    assert_eq!(
        prepare(&root, &unacknowledged, alive).unwrap_err(),
        DamosDenial::IrreversibleNotAcknowledged
    );
    let mut unbounded = request(DamosAction::Pageout);
    unbounded.quota.ms = 0;
    assert_eq!(
        prepare(&root, &unbounded, alive).unwrap_err(),
        DamosDenial::QuotaUnbounded
    );
    unbounded.quota.ms = 500;
    assert_eq!(
        prepare(&root, &unbounded, alive).unwrap_err(),
        DamosDenial::QuotaUnbounded
    );
    let committed = prepare(&root, &request(DamosAction::Pageout), alive)
        .unwrap()
        .commit()
        .unwrap();
    assert_eq!(
        committed.restore(),
        DamosRestore::Cleared {
            irreversible_effect_possible: true
        },
        "configuration restores; the reclaim already done does not"
    );
}

#[test]
fn another_owners_kdamond_and_a_reused_pid_are_never_used() {
    let busy = admin("busy", "on", 1);
    assert_eq!(
        prepare(&busy, &request(DamosAction::Stat), alive).unwrap_err(),
        DamosDenial::NoIdleKdamond
    );
    let configured = admin("configured", "off", 2);
    assert_eq!(
        prepare(&configured, &request(DamosAction::Stat), alive).unwrap_err(),
        DamosDenial::NoIdleKdamond
    );
    let idle = admin("reuse", "off", 0);
    assert_eq!(
        prepare(&idle, &request(DamosAction::Stat), |_| Some(1)).unwrap_err(),
        DamosDenial::TargetIdentity
    );
    let absent = std::env::temp_dir().join("stutter-damos-absent");
    assert_eq!(
        prepare(&absent, &request(DamosAction::Stat), alive).unwrap_err(),
        DamosDenial::Unavailable
    );
    let mut region = request(DamosAction::Stat);
    region.min_region_bytes = 0;
    assert_eq!(
        prepare(&idle, &region, alive).unwrap_err(),
        DamosDenial::BadRegion
    );
}

#[test]
fn restore_leaves_a_kdamond_someone_else_reconfigured() {
    let root = admin("takeover", "off", 0);
    let committed = prepare(&root, &request(DamosAction::Stat), alive)
        .unwrap()
        .commit()
        .unwrap();
    fs::write(
        root.join("kdamonds/0/contexts/0/targets/0/pid_target"),
        "999",
    )
    .unwrap();
    assert_eq!(committed.restore(), DamosRestore::ExternallyChanged);
    assert_eq!(
        read(&root, "0/state"),
        "on",
        "not stopped: it is no longer ours"
    );
}

fn committed_irreversible(name: &str) -> (PathBuf, DamosRecord) {
    let root = admin(name, "off", 0);
    fs::write(root.join("kdamonds/0/pid"), "100").unwrap();
    let committed = prepare(&root, &request(DamosAction::Pageout), alive)
        .unwrap()
        .commit()
        .unwrap();
    let record = committed.record();
    assert_eq!(record.task_start_ticks, 777);
    assert_eq!(record.kdamond_pid.as_deref(), Some("100"));
    (root, record)
}

#[test]
fn recovery_leaves_a_scheme_whose_quota_or_bounds_were_changed_by_someone_else() {
    for (file, value) in [
        ("quotas/bytes", "1"),
        ("quotas/ms", "9"),
        ("quotas/reset_interval_ms", "500"),
        ("access_pattern/sz/min", "8192"),
        ("access_pattern/sz/max", "1"),
    ] {
        let (root, record) = committed_irreversible(&format!("fp-{}", file.replace('/', "-")));
        fs::write(
            root.join("kdamonds/0/contexts/0/schemes/0").join(file),
            value,
        )
        .unwrap();
        assert_eq!(teardown_recorded(&record), DamosRestore::ExternallyChanged);
        assert_eq!(
            read(&root, "0/state"),
            "on",
            "{file}: not ours, not stopped"
        );
    }
    let (root, record) = committed_irreversible("fp-operations");
    fs::write(root.join("kdamonds/0/contexts/0/operations"), "paddr").unwrap();
    assert_eq!(teardown_recorded(&record), DamosRestore::ExternallyChanged);
}

#[test]
fn recovery_leaves_a_replaced_kdamond_even_when_every_value_matches() {
    // A different worker thread behind the same sysfs path.
    let (root, record) = committed_irreversible("replaced-worker");
    fs::write(root.join("kdamonds/0/pid"), "101").unwrap();
    assert_eq!(teardown_recorded(&record), DamosRestore::ExternallyChanged);
    assert_eq!(read(&root, "0/state"), "on");

    // A different sysfs object (new inode) carrying an identical configuration.
    let (root, record) = committed_irreversible("replaced-object");
    let original = root.join("kdamonds/0");
    let aside = root.join("kdamonds/0-original");
    fs::rename(&original, &aside).unwrap();
    for (from, to) in walk(&aside) {
        let target = original.join(to);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(from, target).unwrap();
    }
    assert_eq!(teardown_recorded(&record), DamosRestore::ExternallyChanged);
    assert_eq!(read(&root, "0/state"), "on");
}

fn walk(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    fn inner(base: &Path, dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                inner(base, &path, out);
            } else {
                out.push((path.clone(), path.strip_prefix(base).unwrap().to_owned()));
            }
        }
    }
    let mut out = Vec::new();
    inner(dir, dir, &mut out);
    out
}

#[test]
fn a_record_from_before_identity_was_recorded_is_never_acted_on() {
    let (root, mut record) = committed_irreversible("legacy");
    record.task_start_ticks = 0;
    record.quota_ms = 0;
    record.quota_bytes = 0;
    record.quota_reset_interval_ms = 0;
    record.min_region_bytes = 0;
    record.max_region_bytes = 0;
    record.kdamond_dev = 0;
    record.kdamond_ino = 0;
    record.kdamond_pid = None;
    assert_eq!(teardown_recorded(&record), DamosRestore::ExternallyChanged);
    assert_eq!(read(&root, "0/state"), "on");
}

#[test]
fn an_exact_record_tears_the_scheme_down_after_a_crash() {
    let (root, record) = committed_irreversible("exact");
    assert_eq!(
        teardown_recorded(&record),
        DamosRestore::Cleared {
            irreversible_effect_possible: true
        }
    );
    assert_eq!(read(&root, "0/state"), "off");
    // Idempotent: the same object, now idle and empty, is already torn down.
    assert!(matches!(
        teardown_recorded(&record),
        DamosRestore::Cleared { .. }
    ));
}

fn with_goal(metric: DamosGoalMetric, target: u64) -> DamosRequest {
    DamosRequest {
        goal: Some(DamosGoal {
            metric,
            target_value: target,
        }),
        ..request(DamosAction::Stat)
    }
}

#[test]
fn a_quota_goal_is_written_read_back_fed_and_tears_down_with_the_scheme() {
    let root = admin("goal", "off", 0);
    let committed = prepare(&root, &with_goal(DamosGoalMetric::UserInput, 500), alive)
        .unwrap()
        .commit()
        .unwrap();
    let record = committed.record();
    assert_eq!(record.goal_metric.as_deref(), Some("user_input"));
    let goals = "0/contexts/0/schemes/0/quotas/goals";
    assert_eq!(read(&root, &format!("{goals}/nr_goals")), "1");
    assert_eq!(
        read(&root, &format!("{goals}/0/target_metric")),
        "user_input"
    );
    assert_eq!(read(&root, &format!("{goals}/0/target_value")), "500");
    feed_goal_with(&record, 321, &alive).unwrap();
    assert_eq!(read(&root, &format!("{goals}/0/current_value")), "321");
    assert!(matches!(committed.restore(), DamosRestore::Cleared { .. }));
    assert_eq!(read(&root, "0/state"), "off");
}

#[test]
fn goals_are_denied_without_a_target_or_a_quota_and_feeding_needs_the_exact_owner() {
    let root = admin("goal-deny", "off", 0);
    for request in [
        with_goal(DamosGoalMetric::UserInput, 0),
        DamosRequest {
            quota: DamosQuota {
                ms: 0,
                bytes: 0,
                reset_interval_ms: 1000,
            },
            ..with_goal(DamosGoalMetric::UserInput, 5)
        },
    ] {
        assert_eq!(
            prepare(&root, &request, alive).unwrap_err(),
            DamosDenial::BadGoal
        );
    }
    // A scheme with no goal, or a kernel-measured one, has nothing to feed.
    let (_plain_root, plain) = committed_irreversible("goal-plain");
    assert_eq!(
        feed_goal_with(&plain, 1, &alive).unwrap_err(),
        DamosFeedbackError::NoUserInputGoal
    );
    let psi_root = admin("goal-psi", "off", 0);
    let psi = prepare(
        &psi_root,
        &with_goal(DamosGoalMetric::SomeMemPsiUs, 9),
        alive,
    )
    .unwrap()
    .commit()
    .unwrap()
    .record();
    assert_eq!(
        feed_goal_with(&psi, 1, &alive).unwrap_err(),
        DamosFeedbackError::NoUserInputGoal
    );
    // Someone retargeted the goal: not ours, nothing is written or torn down.
    let fed_root = admin("goal-takeover", "off", 0);
    let fed = prepare(
        &fed_root,
        &with_goal(DamosGoalMetric::UserInput, 500),
        alive,
    )
    .unwrap()
    .commit()
    .unwrap()
    .record();
    fs::write(
        fed_root.join("kdamonds/0/contexts/0/schemes/0/quotas/goals/0/target_value"),
        "1",
    )
    .unwrap();
    assert_eq!(
        feed_goal_with(&fed, 5, &alive).unwrap_err(),
        DamosFeedbackError::NotOurs
    );
    assert_eq!(teardown_recorded(&fed), DamosRestore::ExternallyChanged);
}

#[test]
fn a_gone_or_reused_target_pid_is_handled_explicitly_in_feeding_and_in_teardown() {
    let gone = |_: u32| None;
    let reused = |_: u32| Some(999_999_u64);
    // Feeding is for the recorded process only.
    let root = admin("target-feed", "off", 0);
    let record = prepare(&root, &with_goal(DamosGoalMetric::UserInput, 500), alive)
        .unwrap()
        .commit()
        .unwrap()
        .record();
    assert_eq!(
        feed_goal_with(&record, 1, &gone),
        Err(DamosFeedbackError::TargetGone)
    );
    assert_eq!(
        feed_goal_with(&record, 1, &reused),
        Err(DamosFeedbackError::TargetReplaced)
    );
    assert_eq!(feed_goal_with(&record, 1, &alive), Ok(()));

    // An exited target is still torn down as before.
    let (root_gone, record_gone) = committed_irreversible("target-gone");
    assert_eq!(
        teardown_recorded_with(&record_gone, &gone),
        DamosRestore::Cleared {
            irreversible_effect_possible: true
        }
    );
    assert_eq!(read(&root_gone, "0/state"), "off");

    // A pid that now names another process is cleared but reported, never taken for the target.
    let (root_reused, record_reused) = committed_irreversible("target-reused");
    assert_eq!(
        teardown_recorded_with(&record_reused, &reused),
        DamosRestore::ClearedTargetReplaced {
            irreversible_effect_possible: true
        }
    );
    assert_eq!(read(&root_reused, "0/state"), "off");

    // The same process still being alive is the ordinary case.
    let (_root_same, record_same) = committed_irreversible("target-same");
    assert!(matches!(
        teardown_recorded_with(&record_same, &alive),
        DamosRestore::Cleared { .. }
    ));
}
