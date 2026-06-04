use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use serde::{Deserialize, Serialize};
use stutter_core::ids::{Pid, Tid};

pub(crate) use super::action_decision::ActionStatus;
use super::{
    Profile, ProfileRule,
    action_decision::{
        ProfileAffinityDecision, ProfileIoniceDecision, ProfileNiceDecision,
        ProfileTaskActionDecision, decide_profile_task_actions,
    },
    matching::{CommField, CommPatternHit, RuleMatchEvidence, matching_profile_rule_with_evidence},
};
use crate::{
    affinity::{self, CpuMask},
    process_tree::{self, TargetSnapshot, TaskInfo},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProfileExplainReport {
    pub schema_version: u32,
    pub profile: String,
    pub tree_pid: Option<Pid>,

    pub snapshot_tasks: usize,
    pub matched_tasks: usize,
    pub unmatched_tasks: usize,

    pub pending_unique_tasks: usize,
    pub pending_affinity: usize,
    pub pending_nice: usize,
    pub pending_ionice: usize,

    pub rules: Vec<ProfileRuleExplain>,
    pub unmatched: ProfileUnmatchedExplain,

    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProfileRuleExplain {
    pub rule_index: usize,

    pub actions: ProfileRuleActionExplain,
    pub match_class: Vec<String>,
    pub match_comm: Vec<String>,

    pub matched_tasks: usize,
    pub pending_unique_tasks: usize,
    pub pending_affinity: usize,
    pub pending_nice: usize,
    pub pending_ionice: usize,

    pub already_satisfied_tasks: usize,
    pub skipped_tasks: usize,

    pub match_basis: MatchBasisCounts,

    pub classes: BTreeMap<String, usize>,
    pub top_thread_comms: BTreeMap<String, usize>,
    pub top_process_comms: BTreeMap<String, usize>,

    pub broad_process_comm_captured_thread_comms: BTreeMap<String, usize>,

    pub tasks: Vec<ProfileTaskExplain>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProfileRuleActionExplain {
    pub affinity: Option<String>,
    pub nice: Option<i32>,
    pub ionice: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProfileTaskExplain {
    pub tid: Tid,
    pub process_pid: Pid,

    pub comm: String,
    pub process_comm: String,
    pub class: String,

    pub matched_rule_index: usize,
    pub match_evidence: RuleMatchEvidenceDto,

    pub affinity: Option<ActionDecisionDto>,
    pub nice: Option<ActionDecisionDto>,
    pub ionice: Option<ActionDecisionDto>,

    pub pending: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RuleMatchEvidenceDto {
    pub matched_class: bool,
    pub comm_hits: Vec<CommPatternHitDto>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CommPatternHitDto {
    pub field: CommFieldDto,
    pub pattern: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CommFieldDto {
    TaskComm,
    ProcessComm,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ActionDecisionDto {
    pub status: ActionStatus,
    pub current: Option<String>,
    pub desired: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MatchBasisCounts {
    pub class_only: usize,
    pub task_comm: usize,
    pub process_comm: usize,
    pub both_comm_fields: usize,
    pub catch_all: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProfileUnmatchedExplain {
    pub tasks: usize,
    pub classes: BTreeMap<String, usize>,
    pub top_thread_comms: BTreeMap<String, usize>,
    pub top_process_comms: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileExplainOptions {
    pub tree_pid: Option<Pid>,
    pub include_tasks: bool,
}

impl Default for ProfileExplainOptions {
    fn default() -> Self {
        Self {
            tree_pid: None,
            include_tasks: true,
        }
    }
}

pub(crate) fn explain_profile_for_tree(
    tree_pid: u32,
    profile: &Profile,
) -> anyhow::Result<ProfileExplainReport> {
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );
    explain_profile_for_snapshot(
        profile,
        &snapshot,
        ProfileExplainOptions {
            tree_pid: Some(Pid::new(tree_pid)),
            ..ProfileExplainOptions::default()
        },
        affinity::read_allowed_mask,
        |tid| crate::actions::nice::read_task_nice(tid.as_u32()),
        |tid| crate::actions::ioprio::read_task_ioprio(tid.as_u32()),
    )
}

pub(crate) fn explain_profile_for_snapshot<FA, FN, FI>(
    profile: &Profile,
    snapshot: &TargetSnapshot,
    options: ProfileExplainOptions,
    mut read_allowed_mask: FA,
    mut read_nice: FN,
    mut read_ioprio: FI,
) -> anyhow::Result<ProfileExplainReport>
where
    FA: FnMut(Tid) -> io::Result<CpuMask>,
    FN: FnMut(Tid) -> anyhow::Result<i32>,
    FI: FnMut(Tid) -> anyhow::Result<i32>,
{
    let mut report = ProfileExplainReport {
        schema_version: 1,
        profile: profile.name.clone(),
        tree_pid: options.tree_pid,
        snapshot_tasks: snapshot.tasks.len(),
        matched_tasks: 0,
        unmatched_tasks: 0,
        pending_unique_tasks: 0,
        pending_affinity: 0,
        pending_nice: 0,
        pending_ionice: 0,
        rules: profile
            .rules
            .iter()
            .enumerate()
            .map(|(rule_index, rule)| empty_rule_explain(rule_index, rule))
            .collect(),
        unmatched: ProfileUnmatchedExplain::default(),
        warnings: Vec::new(),
    };

    let mut pending_tids = BTreeSet::<Tid>::new();

    for task in snapshot.tasks.values() {
        let Some((rule_index, rule, evidence)) = matching_profile_rule_with_evidence(task, profile)
        else {
            report.unmatched.tasks += 1;
            increment(&mut report.unmatched.classes, task.class.as_str());
            increment(&mut report.unmatched.top_thread_comms, &task.comm);
            increment(&mut report.unmatched.top_process_comms, &task.process_comm);
            continue;
        };

        let action_decision = decide_profile_task_actions(
            task,
            rule,
            &mut read_allowed_mask,
            &mut read_nice,
            &mut read_ioprio,
        )?;
        let rule_summary = &mut report.rules[rule_index];

        rule_summary.matched_tasks += 1;
        increment(&mut rule_summary.classes, task.class.as_str());
        increment(&mut rule_summary.top_thread_comms, &task.comm);
        increment(&mut rule_summary.top_process_comms, &task.process_comm);
        increment_match_basis(&mut rule_summary.match_basis, &evidence);
        increment_broad_process_comm_captures(
            &mut rule_summary.broad_process_comm_captured_thread_comms,
            task,
            &evidence,
        );

        if action_decision.pending() {
            rule_summary.pending_unique_tasks += 1;
            pending_tids.insert(task.task_id());
        }
        if action_decision.affinity_pending() {
            rule_summary.pending_affinity += 1;
        }
        if action_decision.nice_pending() {
            rule_summary.pending_nice += 1;
        }
        if action_decision.ionice_pending() {
            rule_summary.pending_ionice += 1;
        }
        if action_decision_already_satisfied(&action_decision) {
            rule_summary.already_satisfied_tasks += 1;
        }
        if action_decision_skipped(&action_decision) {
            rule_summary.skipped_tasks += 1;
        }

        if options.include_tasks {
            rule_summary
                .tasks
                .push(task_explain(task, rule_index, &evidence, &action_decision));
        }
    }

    for rule in &report.rules {
        report.matched_tasks += rule.matched_tasks;
        report.pending_affinity += rule.pending_affinity;
        report.pending_nice += rule.pending_nice;
        report.pending_ionice += rule.pending_ionice;
        if rule.matched_tasks == 0 {
            report.warnings.push(format!(
                "Rule {} matched 0 tasks. Earlier rules may have matched tasks first; profile matching is first-match-wins.",
                rule.rule_index
            ));
        }
    }
    report.unmatched_tasks = report.unmatched.tasks;
    report.pending_unique_tasks = pending_tids.len();

    Ok(report)
}

fn empty_rule_explain(rule_index: usize, rule: &ProfileRule) -> ProfileRuleExplain {
    ProfileRuleExplain {
        rule_index,
        actions: ProfileRuleActionExplain {
            affinity: rule.affinity.as_ref().map(CpuMask::to_range_string),
            nice: rule.nice,
            ionice: rule.ionice.map(|value| value.label()),
        },
        match_class: rule
            .match_class
            .iter()
            .map(|class| class.as_str().to_owned())
            .collect(),
        match_comm: rule
            .match_comm
            .iter()
            .map(|pattern| pattern.raw().to_owned())
            .collect(),
        matched_tasks: 0,
        pending_unique_tasks: 0,
        pending_affinity: 0,
        pending_nice: 0,
        pending_ionice: 0,
        already_satisfied_tasks: 0,
        skipped_tasks: 0,
        match_basis: MatchBasisCounts::default(),
        classes: BTreeMap::new(),
        top_thread_comms: BTreeMap::new(),
        top_process_comms: BTreeMap::new(),
        broad_process_comm_captured_thread_comms: BTreeMap::new(),
        tasks: Vec::new(),
    }
}

fn task_explain(
    task: &TaskInfo,
    rule_index: usize,
    evidence: &RuleMatchEvidence,
    decision: &ProfileTaskActionDecision,
) -> ProfileTaskExplain {
    ProfileTaskExplain {
        tid: task.task_id(),
        process_pid: task.process_id(),
        comm: task.comm.clone(),
        process_comm: task.process_comm.clone(),
        class: task.class.as_str().to_owned(),
        matched_rule_index: rule_index,
        match_evidence: RuleMatchEvidenceDto::from(evidence),
        affinity: decision.affinity.as_ref().map(ActionDecisionDto::from),
        nice: decision.nice.as_ref().map(ActionDecisionDto::from),
        ionice: decision.ionice.as_ref().map(ActionDecisionDto::from),
        pending: decision.pending(),
    }
}

impl From<&RuleMatchEvidence> for RuleMatchEvidenceDto {
    fn from(evidence: &RuleMatchEvidence) -> Self {
        Self {
            matched_class: evidence.matched_class,
            comm_hits: evidence
                .comm_hits
                .iter()
                .map(CommPatternHitDto::from)
                .collect(),
        }
    }
}

impl From<&CommPatternHit> for CommPatternHitDto {
    fn from(hit: &CommPatternHit) -> Self {
        Self {
            field: match hit.field {
                CommField::TaskComm => CommFieldDto::TaskComm,
                CommField::ProcessComm => CommFieldDto::ProcessComm,
            },
            pattern: hit.pattern.clone(),
            value: hit.value.clone(),
        }
    }
}

impl From<&ProfileAffinityDecision> for ActionDecisionDto {
    fn from(decision: &ProfileAffinityDecision) -> Self {
        Self {
            status: decision.status,
            current: decision.current.as_ref().map(CpuMask::to_range_string),
            desired: decision.desired.as_ref().map(CpuMask::to_range_string),
            reason: decision.reason.clone(),
        }
    }
}

impl From<&ProfileNiceDecision> for ActionDecisionDto {
    fn from(decision: &ProfileNiceDecision) -> Self {
        Self {
            status: decision.status,
            current: decision.current.map(|nice| nice.to_string()),
            desired: decision.desired.map(|nice| nice.to_string()),
            reason: decision.reason.clone(),
        }
    }
}

impl From<&ProfileIoniceDecision> for ActionDecisionDto {
    fn from(decision: &ProfileIoniceDecision) -> Self {
        Self {
            status: decision.status,
            current: decision
                .current
                .map(|value| value.label())
                .or_else(|| decision.current_encoded.map(|value| value.to_string())),
            desired: decision.desired.map(|value| value.label()),
            reason: decision.reason.clone(),
        }
    }
}

fn increment(map: &mut BTreeMap<String, usize>, key: &str) {
    *map.entry(key.to_owned()).or_default() += 1;
}

fn increment_match_basis(counts: &mut MatchBasisCounts, evidence: &RuleMatchEvidence) {
    if evidence.rule_had_match_comm {
        let matched_task_comm = evidence
            .comm_hits
            .iter()
            .any(|hit| hit.field == CommField::TaskComm);
        let matched_process_comm = evidence
            .comm_hits
            .iter()
            .any(|hit| hit.field == CommField::ProcessComm);

        match (matched_task_comm, matched_process_comm) {
            (true, true) => counts.both_comm_fields += 1,
            (true, false) => counts.task_comm += 1,
            (false, true) => counts.process_comm += 1,
            (false, false) => {}
        }
    } else if evidence.rule_had_match_class {
        counts.class_only += 1;
    } else {
        counts.catch_all += 1;
    }
}

fn increment_broad_process_comm_captures(
    captures: &mut BTreeMap<String, usize>,
    task: &TaskInfo,
    evidence: &RuleMatchEvidence,
) {
    let matched_process_comm = evidence
        .comm_hits
        .iter()
        .any(|hit| hit.field == CommField::ProcessComm);
    if matched_process_comm && task.comm != task.process_comm {
        increment(captures, &task.comm);
    }
}

fn action_decision_already_satisfied(decision: &ProfileTaskActionDecision) -> bool {
    let statuses = action_statuses(decision);
    !statuses.is_empty()
        && statuses
            .iter()
            .all(|status| **status == ActionStatus::AlreadySatisfied)
}

fn action_decision_skipped(decision: &ProfileTaskActionDecision) -> bool {
    action_statuses(decision).iter().any(|status| {
        matches!(
            **status,
            ActionStatus::SkippedDeadTask
                | ActionStatus::SkippedIncompleteIdentity
                | ActionStatus::ReadError
        )
    })
}

fn action_statuses(decision: &ProfileTaskActionDecision) -> Vec<&ActionStatus> {
    [
        decision.affinity.as_ref().map(|decision| &decision.status),
        decision.nice.as_ref().map(|decision| &decision.status),
        decision.ionice.as_ref().map(|decision| &decision.status),
    ]
    .into_iter()
    .flatten()
    .collect()
}
