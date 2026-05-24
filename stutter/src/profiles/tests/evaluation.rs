use std::collections::BTreeMap;

use super::{super::*, support::*};

#[test]
fn match_comm_treats_metacharacters_as_literals_unless_slash_delimited() {
    let literal = CompiledPattern::new("KingdomCome.exe".to_owned()).unwrap();
    assert!(literal.matches("KingdomCome.exe"));
    assert!(literal.matches("kingdomcome.exe"));
    assert!(!literal.matches("KingdomComeXexe"));

    let regex = CompiledPattern::new("/KingdomCome[.]exe$/".to_owned()).unwrap();
    assert!(regex.matches("KingdomCome.exe"));
    assert!(!regex.matches("kingdomcome.exe"));
    assert!(!regex.matches("KingdomComeXexe"));

    let literal_bracket = CompiledPattern::new("[".to_owned()).unwrap();
    assert!(literal_bracket.matches("renderer[0]"));
    assert!(CompiledPattern::new("/[/".to_owned()).is_err());
}

#[test]
fn profile_match_class_sees_community_rule_game_class() {
    let class = process_tree::classify_task_with_context(
        "KingdomCome",
        "KingdomCome",
        "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe",
        "/usr/bin/wine",
        "/user.slice/app-steam-379430.scope",
        None,
    );
    let task = TaskInfo {
        tid: 379430.into(),
        process_pid: 379430.into(),
        process_ppid: 1.into(),
        comm: "KingdomCome".into(),
        process_comm: "KingdomCome".into(),
        process_starttime_ticks: Some(379430),
        task_starttime_ticks: Some(379430),
        exe_dev: None,
        exe_ino: None,
        class,
        sched_policy: None,
        from_cgroup: false,
    };
    let profile = Profile {
        name: "game".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0-1").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };

    assert_eq!(task.class, TaskClass::Game);
    assert!(profile_matches_task(&task, &profile));
}

#[test]
fn profile_apply_summary_counts_matching_tasks_and_pending_changes() {
    let task_correct = TaskInfo {
        tid: 7.into(),
        process_pid: 7.into(),
        process_ppid: 1.into(),
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
    let task_pending = TaskInfo {
        tid: 8.into(),
        process_pid: 8.into(),
        process_ppid: 1.into(),
        comm: "WorkerThread".into(),
        process_comm: "game".into(),
        process_starttime_ticks: Some(80),
        task_starttime_ticks: Some(80),
        exe_dev: None,
        exe_ino: None,
        class: TaskClass::Game,
        sched_policy: None,
        from_cgroup: false,
    };
    let task_unmatched = TaskInfo {
        tid: 9.into(),
        process_pid: 9.into(),
        process_ppid: 1.into(),
        comm: "Compositor".into(),
        process_comm: "sway".into(),
        process_starttime_ticks: Some(90),
        task_starttime_ticks: Some(90),
        exe_dev: None,
        exe_ino: None,
        class: TaskClass::Compositor,
        sched_policy: None,
        from_cgroup: false,
    };
    let tasks = BTreeMap::from([(7, task_correct), (8, task_pending), (9, task_unmatched)]);
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

    let summary = profile_apply_summary_with_reader(&tasks, &profile, |tid| match tid {
        7 => Ok(CpuMask::parse("0-1").unwrap()),
        8 => Ok(CpuMask::parse("0").unwrap()),
        9 => Ok(CpuMask::parse("0-1").unwrap()),
        other => panic!("unexpected TID {other}"),
    })
    .unwrap();

    assert_eq!(
        summary,
        ProfileApplySummary {
            checked_tasks: 2,
            pending_changes: 1,
            pending_affinity: 1,
            pending_nice: 0,
            pending_ionice: 0,
        }
    );
}

#[test]
fn profile_evaluation_matches_apply_plan_rule_order_and_masks() {
    let main = test_task(11, TaskClass::Game, "Main");
    let render = test_task(12, TaskClass::GameRenderThread, "RenderThread");
    let worker = test_task(13, TaskClass::Game, "WorkerThread");
    let compositor = test_task(14, TaskClass::Compositor, "kwin_wayland");
    let service = test_task(15, TaskClass::Service, "dbus-daemon");
    let tasks = BTreeMap::from([
        (main.task_id().as_u32(), main.clone()),
        (render.task_id().as_u32(), render.clone()),
        (worker.task_id().as_u32(), worker.clone()),
        (compositor.task_id().as_u32(), compositor.clone()),
        (service.task_id().as_u32(), service.clone()),
    ]);
    let profile = Profile {
        name: "rule-order".to_owned(),
        rules: vec![
            ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: vec![CompiledPattern::new("Main".to_owned()).unwrap()],
            },
            ProfileRule {
                affinity: Some(CpuMask::parse("1").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game, TaskClass::GameRenderThread],
                match_comm: Vec::new(),
            },
            ProfileRule {
                affinity: Some(CpuMask::parse("2").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Compositor],
                match_comm: Vec::new(),
            },
        ],
    };
    let active_tasks = tasks
        .values()
        .map(active_task_from_task)
        .collect::<Vec<_>>();

    let evaluated = evaluate_profile_for_tasks(ProfileEvaluationInput {
        profile: &profile,
        active_tasks: &active_tasks,
        topology: None,
    })
    .into_iter()
    .map(|task| (task.tid, task.requested_mask, task.matched_rule_index))
    .collect::<Vec<_>>();
    assert_eq!(
        evaluated,
        vec![
            (11, "0".to_owned(), 0),
            (12, "1".to_owned(), 1),
            (13, "1".to_owned(), 1),
            (14, "2".to_owned(), 2),
        ]
    );

    let apply_plan = planned_profile_apply_with_readers(
        &tasks,
        &profile,
        None,
        |_| Ok(CpuMask::parse("7").unwrap()),
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();
    let mut apply_decisions = apply_plan
        .affinity_changes
        .iter()
        .map(|change| {
            (
                change.record.tid,
                change.record.applied_mask.to_range_string(),
            )
        })
        .collect::<Vec<_>>();
    apply_decisions.sort_by_key(|(tid, _)| *tid);

    assert_eq!(
        apply_decisions,
        evaluated
            .into_iter()
            .map(|(tid, mask, _)| (tid, mask))
            .collect::<Vec<_>>()
    );
}

#[test]
fn profile_plan_constructs_task_identity_for_priority_targets() {
    let task = test_task(42, TaskClass::Indexer, "indexer");
    let tasks = BTreeMap::from([(42, task)]);
    let profile = Profile {
        name: "priority".to_owned(),
        rules: vec![ProfileRule {
            affinity: None,
            nice: Some(10),
            ionice: None,
            match_class: vec![TaskClass::Indexer],
            match_comm: Vec::new(),
        }],
    };

    let plan = planned_profile_apply_with_readers(
        &tasks,
        &profile,
        None,
        |_| panic!("affinity should not be read"),
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();
    let identity = &plan.nice_groups.get(&10).unwrap()[0];

    assert_eq!(identity.tid.as_u32(), 42);
    assert_eq!(identity.process_pid.map(|pid| pid.as_u32()), Some(42));
    assert_eq!(identity.comm.as_deref(), Some("indexer"));
    assert_eq!(identity.starttime_ticks, Some(421));
}

#[test]
fn profile_apply_summary_counts_priority_actions_without_double_counting_tasks() {
    let task = test_task(42, TaskClass::PackageManager, "pacman");
    let tasks = BTreeMap::from([(42, task)]);
    let profile = Profile {
        name: "priority".to_owned(),
        rules: vec![ProfileRule {
            affinity: None,
            nice: Some(10),
            ionice: Some(IoPrioValue::idle()),
            match_class: vec![TaskClass::PackageManager],
            match_comm: Vec::new(),
        }],
    };

    let summary = profile_apply_summary_with_readers(
        &tasks,
        &profile,
        |_| panic!("affinity should not be read"),
        |_| Ok(0),
        |_| Ok(IoPrioValue::best_effort(4).encode().unwrap()),
    )
    .unwrap();

    assert_eq!(
        summary,
        ProfileApplySummary {
            checked_tasks: 1,
            pending_changes: 1,
            pending_affinity: 0,
            pending_nice: 1,
            pending_ionice: 1,
        }
    );
}

#[test]
fn profile_matched_task_count_counts_only_matching_rules() {
    let game_task = TaskInfo {
        tid: 7.into(),
        process_pid: 7.into(),
        process_ppid: 1.into(),
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
    let compositor_task = TaskInfo {
        tid: 8.into(),
        process_pid: 8.into(),
        process_ppid: 1.into(),
        comm: "Compositor".into(),
        process_comm: "sway".into(),
        process_starttime_ticks: Some(80),
        task_starttime_ticks: Some(80),
        exe_dev: None,
        exe_ino: None,
        class: TaskClass::Compositor,
        sched_policy: None,
        from_cgroup: false,
    };
    let tasks = BTreeMap::from([(7, game_task), (8, compositor_task)]);
    let profile = Profile {
        name: "game-render".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: vec![CompiledPattern::new("RenderThread".to_owned()).unwrap()],
        }],
    };

    assert_eq!(profile_matched_task_count(&tasks, &profile), 1);
    assert!(profile_matches_task(tasks.get(&7).unwrap(), &profile));
    assert!(!profile_matches_task(tasks.get(&8).unwrap(), &profile));
}
