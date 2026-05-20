//! Tests for profile parsing, matching, planning, and warning behavior.
//!
//! Owns profile regression tests and test-only fixtures. Does not own production profile parsing,
//! matching, application, rendering, or warning logic.

use std::path::Path;

use super::*;

fn test_task(tid: u32, class: TaskClass, comm: &str) -> TaskInfo {
    TaskInfo {
        tid,
        process_pid: tid,
        process_ppid: 1,
        comm: comm.into(),
        process_comm: "process".into(),
        process_starttime_ticks: Some(u64::from(tid) * 10),
        task_starttime_ticks: Some(u64::from(tid) * 10 + 1),
        exe_dev: None,
        exe_ino: None,
        class,
        sched_policy: None,
        from_cgroup: false,
    }
}

fn active_task_from_task(task: &TaskInfo) -> crate::autotune::observation::ActiveTaskSnapshot {
    crate::autotune::observation::ActiveTaskSnapshot {
        tid: task.tid,
        process_pid: task.process_pid,
        comm: task.comm.clone(),
        class: task.class,
        process_starttime_ticks: task.process_starttime_ticks,
        task_starttime_ticks: task.task_starttime_ticks,
        cgroup_path: None,
    }
}

#[test]
fn parses_minimal_profile() {
    let profiles = parse_profiles(
        r#"
        [[profile]]
        name = "kcd # not a comment"

        [[profile.rules]]
        affinity = "0-3"
        match_class = ["Game"]
        match_comm = ["RenderThread", "Main"]
        "#,
    )
    .unwrap();

    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "kcd # not a comment");
    let rule = &profiles[0].rules[0];
    assert_eq!(rule.affinity.as_ref().unwrap().to_range_string(), "0-3");
    assert_eq!(rule.match_class, vec![TaskClass::Game]);
    let comm_patterns = rule
        .match_comm
        .iter()
        .map(CompiledPattern::raw)
        .collect::<Vec<_>>();
    assert_eq!(comm_patterns, vec!["RenderThread", "Main"]);
}

#[test]
fn render_profiles_toml_outputs_profile_rules() {
    let profile = Profile {
        name: "generated \"profile\"".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0-1").unwrap()),
            nice: Some(5),
            ionice: Some(IoPrioValue::idle()),
            match_class: vec![TaskClass::Game, TaskClass::GameRenderThread],
            match_comm: vec![
                CompiledPattern::new("RenderThread".to_owned()).unwrap(),
                CompiledPattern::new("Main".to_owned()).unwrap(),
            ],
        }],
    };

    let toml = render_profiles_toml(&[profile]);

    assert!(toml.contains("[[profile]]"));
    assert!(toml.contains("name = \"generated \\\"profile\\\"\""));
    assert!(toml.contains("[[profile.rules]]"));
    assert!(toml.contains("affinity = \"0-1\""));
    assert!(toml.contains("nice = 5"));
    assert!(toml.contains("ionice = \"idle\""));
    assert!(toml.contains("match_class = [\"Game\", \"GameRenderThread\"]"));
    assert!(toml.contains("match_comm = [\"RenderThread\", \"Main\"]"));
}

#[test]
fn profile_parser_accepts_online_affinity() {
    let profiles = parse_profiles(
        r#"
        [[profile]]
        name = "baseline-online"

        [[profile.rules]]
        affinity = "online"
        match_class = ["Game"]
        "#,
    )
    .unwrap();

    assert_eq!(profiles.len(), 1);
    assert!(!profiles[0].rules[0].affinity.as_ref().unwrap().is_empty());
}

#[test]
fn profile_parser_accepts_nice_only_rule() {
    let profiles = parse_profiles(
        r#"
        [[profile]]
        name = "background"

        [[profile.rules]]
        match_class = ["Indexer"]
        nice = 10
        "#,
    )
    .unwrap();

    let rule = &profiles[0].rules[0];
    assert!(rule.affinity.is_none());
    assert_eq!(rule.nice, Some(10));
    assert_eq!(rule.ionice, None);
}

#[test]
fn profile_parser_accepts_ionice_only_rule() {
    let profiles = parse_profiles(
        r#"
        [[profile]]
        name = "background"

        [[profile.rules]]
        match_class = ["PackageManager"]
        ionice = "idle"
        "#,
    )
    .unwrap();

    let rule = &profiles[0].rules[0];
    assert!(rule.affinity.is_none());
    assert_eq!(rule.nice, None);
    assert_eq!(rule.ionice, Some(IoPrioValue::idle()));
}

#[test]
fn profile_parser_accepts_combined_affinity_nice_ionice_rule() {
    let profiles = parse_profiles(
        r#"
        [[profile]]
        name = "game-latency"

        [[profile.rules]]
        match_class = ["Game", "GameRenderThread"]
        affinity = "0-3"
        nice = -5
        ionice = "be:2"
        "#,
    )
    .unwrap();

    let rule = &profiles[0].rules[0];
    assert_eq!(rule.affinity.as_ref().unwrap().to_range_string(), "0-3");
    assert_eq!(rule.nice, Some(-5));
    assert_eq!(rule.ionice, Some(IoPrioValue::best_effort(2)));
}

#[test]
fn profile_parser_rejects_invalid_nice_range() {
    let err = parse_profiles(
        r#"
        [[profile]]
        name = "bad"

        [[profile.rules]]
        nice = 20
        "#,
    )
    .unwrap_err();

    assert!(format!("{err:#}").contains("outside Linux range"));
}

#[test]
fn profile_parser_rejects_invalid_ionice_strings() {
    for ionice in ["best-effort", "realtime", "be:8", "rt:9", "idle:4"] {
        let err = parse_profiles(&format!(
            r#"
            [[profile]]
            name = "bad"

            [[profile.rules]]
            ionice = "{ionice}"
            "#
        ))
        .unwrap_err();

        assert!(
            format!("{err:#}").contains("ionice") || format!("{err:#}").contains("I/O priority")
        );
    }
}

#[test]
fn profile_parser_rejects_rule_with_no_action_fields() {
    let err = parse_profiles(
        r#"
        [[profile]]
        name = "bad"

        [[profile.rules]]
        match_class = ["Game"]
        "#,
    )
    .unwrap_err();

    assert!(format!("{err:#}").contains("at least one action field"));
}

#[test]
fn invalid_symbolic_affinity_fails_clearly() {
    let err = parse_profiles(
        r#"
        [[profile]]
        name = "bad"

        [[profile.rules]]
        affinity = "all"
        match_class = ["Game"]
        "#,
    )
    .unwrap_err();

    assert!(err.to_string().contains("invalid CPU id"));
}

#[test]
fn examples_profile_file_parses() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir
        .parent()
        .unwrap()
        .join("examples/profiles/common-game-layouts.toml");
    let profiles = load_profiles(&path).unwrap();

    assert!(!profiles.is_empty());
    assert!(
        profiles
            .iter()
            .any(|profile| profile.name == "baseline-online")
    );
}

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
        tid: 379430,
        process_pid: 379430,
        process_ppid: 1,
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
    let task_pending = TaskInfo {
        tid: 8,
        process_pid: 8,
        process_ppid: 1,
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
        tid: 9,
        process_pid: 9,
        process_ppid: 1,
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
        (main.tid, main.clone()),
        (render.tid, render.clone()),
        (worker.tid, worker.clone()),
        (compositor.tid, compositor.clone()),
        (service.tid, service.clone()),
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

    assert_eq!(identity.tid, 42);
    assert_eq!(identity.process_pid, Some(42));
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
fn profile_matched_task_count_counts_only_matching_rules() {
    let game_task = TaskInfo {
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
    let compositor_task = TaskInfo {
        tid: 8,
        process_pid: 8,
        process_ppid: 1,
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

#[test]
fn profile_offline_cpu_warnings_detects_rule_with_offline_cpus() {
    let profile = Profile {
        name: "test".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0-3").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };
    let online = CpuMask::parse("0-1").unwrap();

    let warnings = profile_offline_cpu_warnings(&profile, &online);

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].rule_index, 0);
    assert_eq!(warnings[0].requested, "0-3");
    assert_eq!(warnings[0].online, "0-1");
}

#[test]
fn profile_offline_cpu_warnings_empty_when_subset() {
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
    let online = CpuMask::parse("0-3").unwrap();

    let warnings = profile_offline_cpu_warnings(&profile, &online);

    assert!(warnings.is_empty());
}

#[test]
fn profile_offline_cpu_warnings_multiple_rules_report_correct_indexes() {
    let profile = Profile {
        name: "test".to_owned(),
        rules: vec![
            ProfileRule {
                affinity: Some(CpuMask::parse("0-1").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            },
            ProfileRule {
                affinity: Some(CpuMask::parse("2-3").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::GameHelper],
                match_comm: Vec::new(),
            },
        ],
    };
    let online = CpuMask::parse("0-1").unwrap();

    let warnings = profile_offline_cpu_warnings(&profile, &online);

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].rule_index, 1);
    assert_eq!(warnings[0].requested, "2-3");
    assert_eq!(warnings[0].online, "0-1");
}

#[test]
fn profile_rule_overlap_warnings_broad_game_before_specific_render_thread_warns() {
    let profile = parse_profiles(
        r#"
        [[profile]]
        name = "test"

        [[profile.rules]]
        match_class = ["Game"]
        affinity = "0-7"

        [[profile.rules]]
        match_comm = ["RenderThread"]
        affinity = "2-5"
        "#,
    )
    .unwrap()
    .pop()
    .unwrap();

    let warnings = profile_rule_overlap_warnings(&profile.rules);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].earlier_rule, 0);
    assert_eq!(warnings[0].later_rule, 1);
}

#[test]
fn profile_rule_overlap_warnings_disjoint_classes_do_not_warn() {
    let profile = parse_profiles(
        r#"
        [[profile]]
        name = "test"

        [[profile.rules]]
        match_class = ["Game"]
        affinity = "0-7"

        [[profile.rules]]
        match_class = ["Compositor"]
        affinity = "8-11"
        "#,
    )
    .unwrap()
    .pop()
    .unwrap();

    let warnings = profile_rule_overlap_warnings(&profile.rules);
    assert!(warnings.is_empty());
}

#[test]
fn profile_rule_overlap_warnings_catch_all_before_anything_warns() {
    let profile = parse_profiles(
        r#"
        [[profile]]
        name = "test"

        [[profile.rules]]
        affinity = "0-7"

        [[profile.rules]]
        match_class = ["Game"]
        affinity = "2-5"
        "#,
    )
    .unwrap()
    .pop()
    .unwrap();

    let warnings = profile_rule_overlap_warnings(&profile.rules);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].earlier_rule, 0);
    assert_eq!(warnings[0].later_rule, 1);
}

#[test]
fn profile_rule_overlap_warnings_exact_same_comm_warns() {
    let profile = parse_profiles(
        r#"
        [[profile]]
        name = "test"

        [[profile.rules]]
        match_comm = ["RenderThread"]
        affinity = "0-3"

        [[profile.rules]]
        match_comm = ["RenderThread"]
        affinity = "4-7"
        "#,
    )
    .unwrap()
    .pop()
    .unwrap();

    let warnings = profile_rule_overlap_warnings(&profile.rules);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].earlier_rule, 0);
    assert_eq!(warnings[0].later_rule, 1);
}
