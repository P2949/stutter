use std::collections::BTreeMap;

use super::{super::*, support::*};

#[test]
fn profile_apply_cache_invalidates_when_desired_nice_changes() {
    let task = test_task(42, TaskClass::Indexer, "indexer");
    let tasks = BTreeMap::from([(42, task)]);
    let mut cache = ProfileApplyCache::default();
    let mut nice_reads = 0;

    let profile_nice_5 = Profile {
        name: "priority".to_owned(),
        rules: vec![ProfileRule {
            affinity: None,
            nice: Some(5),
            ionice: None,
            match_class: vec![TaskClass::Indexer],
            match_comm: Vec::new(),
        }],
    };
    let profile_nice_6 = Profile {
        name: "priority".to_owned(),
        rules: vec![ProfileRule {
            affinity: None,
            nice: Some(6),
            ionice: None,
            match_class: vec![TaskClass::Indexer],
            match_comm: Vec::new(),
        }],
    };

    let first = planned_profile_apply_with_readers(
        &tasks,
        &profile_nice_5,
        Some(&mut cache),
        |_| panic!("affinity should not be read"),
        |_| {
            nice_reads += 1;
            Ok(5)
        },
        |_| Ok(0),
    )
    .unwrap();
    assert!(first.is_empty());
    assert_eq!(nice_reads, 1);

    let second = planned_profile_apply_with_readers(
        &tasks,
        &profile_nice_5,
        Some(&mut cache),
        |_| panic!("affinity should not be read"),
        |_| {
            nice_reads += 1;
            Ok(5)
        },
        |_| Ok(0),
    )
    .unwrap();
    assert!(second.is_empty());
    assert_eq!(nice_reads, 1);

    let third = planned_profile_apply_with_readers(
        &tasks,
        &profile_nice_6,
        Some(&mut cache),
        |_| panic!("affinity should not be read"),
        |_| {
            nice_reads += 1;
            Ok(5)
        },
        |_| Ok(0),
    )
    .unwrap();
    assert_eq!(third.summary.pending_nice, 1);
    assert_eq!(nice_reads, 2);
}

#[test]
fn profile_apply_cache_invalidates_when_desired_ionice_changes() {
    let task = test_task(42, TaskClass::PackageManager, "pacman");
    let tasks = BTreeMap::from([(42, task)]);
    let mut cache = ProfileApplyCache::default();
    let mut ionice_reads = 0;
    let idle = IoPrioValue::idle();
    let best_effort = IoPrioValue::best_effort(6);

    let profile_idle = Profile {
        name: "priority".to_owned(),
        rules: vec![ProfileRule {
            affinity: None,
            nice: None,
            ionice: Some(idle),
            match_class: vec![TaskClass::PackageManager],
            match_comm: Vec::new(),
        }],
    };
    let profile_be = Profile {
        name: "priority".to_owned(),
        rules: vec![ProfileRule {
            affinity: None,
            nice: None,
            ionice: Some(best_effort),
            match_class: vec![TaskClass::PackageManager],
            match_comm: Vec::new(),
        }],
    };

    let first = planned_profile_apply_with_readers(
        &tasks,
        &profile_idle,
        Some(&mut cache),
        |_| panic!("affinity should not be read"),
        |_| Ok(0),
        |_| {
            ionice_reads += 1;
            Ok(idle.encode().unwrap())
        },
    )
    .unwrap();
    assert!(first.is_empty());
    assert_eq!(ionice_reads, 1);

    let second = planned_profile_apply_with_readers(
        &tasks,
        &profile_idle,
        Some(&mut cache),
        |_| panic!("affinity should not be read"),
        |_| Ok(0),
        |_| {
            ionice_reads += 1;
            Ok(idle.encode().unwrap())
        },
    )
    .unwrap();
    assert!(second.is_empty());
    assert_eq!(ionice_reads, 1);

    let third = planned_profile_apply_with_readers(
        &tasks,
        &profile_be,
        Some(&mut cache),
        |_| panic!("affinity should not be read"),
        |_| Ok(0),
        |_| {
            ionice_reads += 1;
            Ok(idle.encode().unwrap())
        },
    )
    .unwrap();
    assert_eq!(third.summary.pending_ionice, 1);
    assert_eq!(ionice_reads, 2);
}

#[test]
fn profile_apply_cache_skips_unchanged_known_correct_tasks() {
    let task = TaskInfo {
        tid: 7,
        process_pid: 7,
        process_ppid: 1,
        comm: "RenderThread".into(),
        process_comm: "game".into(),
        process_starttime_ticks: Some(70),
        task_starttime_ticks: Some(70),
        exe_dev: None,
        exe_ino: None,
        class: TaskClass::Game,
        sched_policy: None,
        from_cgroup: false,
    };
    let tasks = BTreeMap::from([(7, task)]);
    let profile = Profile {
        name: "test".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0-1").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };
    let mut cache = ProfileApplyCache::default();
    let mut reads = 0;

    let first = planned_profile_apply_with_readers(
        &tasks,
        &profile,
        Some(&mut cache),
        |tid| {
            reads += 1;
            assert_eq!(tid, 7);
            Ok(CpuMask::parse("0-1").unwrap())
        },
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();
    assert!(first.is_empty());
    assert_eq!(reads, 1);

    let second = planned_profile_apply_with_readers(
        &tasks,
        &profile,
        Some(&mut cache),
        |_| {
            reads += 1;
            Ok(CpuMask::parse("0-1").unwrap())
        },
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();
    assert!(second.is_empty());
    assert_eq!(reads, 1);

    cache.clear();
    let third = planned_profile_apply_with_readers(
        &tasks,
        &profile,
        Some(&mut cache),
        |_| {
            reads += 1;
            Ok(CpuMask::parse("0-1").unwrap())
        },
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();
    assert!(third.is_empty());
    assert_eq!(reads, 2);
}
