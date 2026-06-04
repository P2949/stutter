use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use stutter_core::ids::Tid;

use super::{
    super::{
        action_decision::ActionStatus,
        explain::{ProfileExplainOptions, explain_profile_for_snapshot},
        planned_profile_apply_with_readers, *,
    },
    support::*,
};
use crate::process_tree::{ScanBudgetReport, TargetSnapshot};

fn snapshot_from_tasks(tasks: Vec<TaskInfo>) -> TargetSnapshot {
    TargetSnapshot {
        process_roots: tasks
            .iter()
            .map(TaskInfo::process_id)
            .collect::<BTreeSet<_>>(),
        tasks: tasks
            .into_iter()
            .map(|task| (task.task_id(), task))
            .collect::<BTreeMap<_, _>>(),
        budget_report: ScanBudgetReport::default(),
    }
}

#[test]
fn explain_counts_snapshot_matched_and_unmatched_tasks() {
    let mut main = test_task(10, TaskClass::Game, "Main");
    main.process_comm = "Main".to_owned();
    let compositor = test_task(11, TaskClass::Compositor, "kwin_wayland");
    let snapshot = snapshot_from_tasks(vec![main, compositor]);
    let profile = Profile {
        name: "counts".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: Vec::new(),
            match_comm: vec![CompiledPattern::new("Main".to_owned()).unwrap()],
        }],
    };

    let report = explain_profile_for_snapshot(
        &profile,
        &snapshot,
        ProfileExplainOptions::default(),
        |_| Ok(CpuMask::parse("1").unwrap()),
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();

    assert_eq!(report.snapshot_tasks, 2);
    assert_eq!(report.matched_tasks, 1);
    assert_eq!(report.unmatched_tasks, 1);
    assert_eq!(report.pending_unique_tasks, 1);
    assert_eq!(report.pending_affinity, 1);
    assert_eq!(report.unmatched.classes.get("Compositor"), Some(&1));
}

#[test]
fn explain_distinguishes_task_comm_and_process_comm_matches() {
    let mut main = test_task(20, TaskClass::Game, "Main");
    main.process_comm = "Main".to_owned();
    let mut render = test_task(21, TaskClass::GameRenderThread, "RenderThread");
    render.process_comm = "Main".to_owned();
    let snapshot = snapshot_from_tasks(vec![main, render]);
    let profile = Profile {
        name: "comm-source".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: Vec::new(),
            match_comm: vec![CompiledPattern::new("Main".to_owned()).unwrap()],
        }],
    };

    let report = explain_profile_for_snapshot(
        &profile,
        &snapshot,
        ProfileExplainOptions::default(),
        |_| Ok(CpuMask::parse("1").unwrap()),
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();
    let rule = &report.rules[0];

    assert_eq!(rule.match_basis.both_comm_fields, 1);
    assert_eq!(rule.match_basis.process_comm, 1);
    assert_eq!(
        rule.broad_process_comm_captured_thread_comms
            .get("RenderThread"),
        Some(&1)
    );
    assert_eq!(rule.tasks[1].match_evidence.comm_hits[0].value, "Main");
}

#[test]
fn explain_counts_classes_and_top_thread_comms_per_rule() {
    let mut render = test_task(30, TaskClass::GameRenderThread, "RenderThread");
    render.process_comm = "Main".to_owned();
    let mut audio = test_task(31, TaskClass::AudioRealtime, "AudioThread");
    audio.process_comm = "Main".to_owned();
    let snapshot = snapshot_from_tasks(vec![render, audio]);
    let profile = Profile {
        name: "classes".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: Vec::new(),
            match_comm: vec![CompiledPattern::new("Main".to_owned()).unwrap()],
        }],
    };

    let report = explain_profile_for_snapshot(
        &profile,
        &snapshot,
        ProfileExplainOptions::default(),
        |_| Ok(CpuMask::parse("1").unwrap()),
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();
    let rule = &report.rules[0];

    assert_eq!(rule.classes.get("GameRenderThread"), Some(&1));
    assert_eq!(rule.classes.get("AudioRealtime"), Some(&1));
    assert_eq!(rule.top_thread_comms.get("RenderThread"), Some(&1));
    assert_eq!(rule.top_process_comms.get("Main"), Some(&2));
}

#[test]
fn explain_marks_affinity_pending_when_current_mask_differs() {
    let task = test_task(40, TaskClass::Game, "Main");
    let snapshot = snapshot_from_tasks(vec![task]);
    let profile = Profile {
        name: "pending".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };

    let report = explain_profile_for_snapshot(
        &profile,
        &snapshot,
        ProfileExplainOptions::default(),
        |_| Ok(CpuMask::parse("1").unwrap()),
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();

    assert_eq!(
        report.rules[0].tasks[0].affinity.as_ref().unwrap().status,
        ActionStatus::Pending
    );
    assert!(report.rules[0].tasks[0].pending);
}

#[test]
fn explain_marks_affinity_already_satisfied_when_mask_matches() {
    let task = test_task(41, TaskClass::Game, "Main");
    let snapshot = snapshot_from_tasks(vec![task]);
    let profile = Profile {
        name: "satisfied".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };

    let report = explain_profile_for_snapshot(
        &profile,
        &snapshot,
        ProfileExplainOptions::default(),
        |_| Ok(CpuMask::parse("0").unwrap()),
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();

    assert_eq!(report.rules[0].already_satisfied_tasks, 1);
    assert_eq!(
        report.rules[0].tasks[0].affinity.as_ref().unwrap().status,
        ActionStatus::AlreadySatisfied
    );
}

#[test]
fn explain_preserves_first_match_wins_rule_index() {
    let mut render = test_task(50, TaskClass::GameRenderThread, "RenderThread");
    render.process_comm = "Main".to_owned();
    let snapshot = snapshot_from_tasks(vec![render]);
    let profile = Profile {
        name: "first-match".to_owned(),
        rules: vec![
            ProfileRule {
                affinity: Some(CpuMask::parse("1-5,7-11").unwrap()),
                nice: None,
                ionice: None,
                match_class: Vec::new(),
                match_comm: vec![CompiledPattern::new("Main".to_owned()).unwrap()],
            },
            ProfileRule {
                affinity: Some(CpuMask::parse("0,6").unwrap()),
                nice: None,
                ionice: None,
                match_class: Vec::new(),
                match_comm: vec![CompiledPattern::new("RenderThread".to_owned()).unwrap()],
            },
        ],
    };

    let report = explain_profile_for_snapshot(
        &profile,
        &snapshot,
        ProfileExplainOptions::default(),
        |_| Ok(CpuMask::parse("0-11").unwrap()),
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();

    assert_eq!(report.rules[0].matched_tasks, 1);
    assert_eq!(report.rules[1].matched_tasks, 0);
    assert_eq!(report.rules[0].tasks[0].matched_rule_index, 0);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("Rule 1 matched 0 tasks"))
    );
}

#[test]
fn explain_serializes_typed_ids_as_numbers() {
    let task = test_task(60, TaskClass::Game, "Main");
    let snapshot = snapshot_from_tasks(vec![task]);
    let profile = Profile {
        name: "ids".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };

    let report = explain_profile_for_snapshot(
        &profile,
        &snapshot,
        ProfileExplainOptions::default(),
        |_| Ok(CpuMask::parse("1").unwrap()),
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();
    let value = serde_json::to_value(&report).unwrap();

    assert_eq!(value["rules"][0]["tasks"][0]["tid"], Value::from(60));
    assert_eq!(
        value["rules"][0]["tasks"][0]["process_pid"],
        Value::from(60)
    );
}

#[test]
fn explain_reports_broad_process_comm_helper_captures() {
    let mut helper = test_task(70, TaskClass::Helper, "dxvk-submit");
    helper.process_comm = "Main".to_owned();
    let snapshot = snapshot_from_tasks(vec![helper]);
    let profile = Profile {
        name: "broad-capture".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: Vec::new(),
            match_comm: vec![CompiledPattern::new("Main".to_owned()).unwrap()],
        }],
    };

    let report = explain_profile_for_snapshot(
        &profile,
        &snapshot,
        ProfileExplainOptions::default(),
        |_| Ok(CpuMask::parse("1").unwrap()),
        |_| Ok(0),
        |_| Ok(0),
    )
    .unwrap();

    assert_eq!(
        report.rules[0]
            .broad_process_comm_captured_thread_comms
            .get("dxvk-submit"),
        Some(&1)
    );
}

#[test]
fn explain_pending_counts_match_apply_planner() {
    let task = test_task(80, TaskClass::Game, "Main");
    let tasks = BTreeMap::from([(Tid::new(80), task.clone())]);
    let snapshot = snapshot_from_tasks(vec![task]);
    let profile = Profile {
        name: "parity".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: Some(5),
            ionice: Some(IoPrioValue::idle()),
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };

    let plan = planned_profile_apply_with_readers(
        &tasks,
        &profile,
        None,
        |_| Ok(CpuMask::parse("1").unwrap()),
        |_| Ok(0),
        |_| Ok(IoPrioValue::best_effort(4).encode().unwrap()),
    )
    .unwrap();
    let report = explain_profile_for_snapshot(
        &profile,
        &snapshot,
        ProfileExplainOptions::default(),
        |_| Ok(CpuMask::parse("1").unwrap()),
        |_| Ok(0),
        |_| Ok(IoPrioValue::best_effort(4).encode().unwrap()),
    )
    .unwrap();

    assert_eq!(plan.summary.pending_changes, report.pending_unique_tasks);
    assert_eq!(plan.summary.pending_affinity, report.pending_affinity);
    assert_eq!(plan.summary.pending_nice, report.pending_nice);
    assert_eq!(plan.summary.pending_ionice, report.pending_ionice);
}
