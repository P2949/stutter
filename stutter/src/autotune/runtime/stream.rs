//! Runtime decision stream DTOs and stdout emission boundary.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

use serde::Serialize;

use super::{
    decision_view::{data_quality_label, decision_label},
    planning::top_denied_reason_for_plan,
};
use crate::{
    actions::SafetyClass,
    autotune::{
        decision::AutotuneDecision,
        observation::AutotuneObservation,
        planner::{CandidateDenyReason, PlanResult, PlannerSummary},
        runtime::AutotuneRuntime,
    },
};

#[derive(Clone, Debug, Serialize)]
pub struct AutotuneDecisionStreamEntry {
    pub unix_nanos: u128,
    pub phase: String,
    pub mode: String,
    pub focus_kind: Option<String>,
    pub focus_confidence: f32,
    pub target_root_pid: Option<u32>,
    pub active_target_count: usize,
    pub situation: String,
    pub situation_confidence: f32,
    pub situation_evidence: Vec<String>,
    pub situation_blockers: Vec<String>,
    pub protected_tasks_count: usize,
    pub candidate_count: usize,
    pub top_denied_reason: Option<String>,
    pub planner: Option<PlannerSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dry_run_plan_files: Vec<AutotuneDryRunPlanFileSummary>,
    pub score_total: u64,
    pub data_quality: String,
    pub data_quality_reason_codes: Vec<String>,
    pub decision: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AutotuneDryRunPlanFileSummary {
    pub candidate_name: String,
    pub action_kind: String,
    pub path: PathBuf,
    pub affected_tasks: usize,
    pub safety_class: SafetyClass,
    pub eligible: bool,
    pub deny_reasons: Vec<CandidateDenyReason>,
}

pub(crate) fn emit_decision_stream_entry(
    entry: &AutotuneDecisionStreamEntry,
) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(entry)?);
    Ok(())
}

impl AutotuneRuntime {
    pub(super) fn stream_entry_from_decision(
        &self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: String,
    ) -> AutotuneDecisionStreamEntry {
        AutotuneDecisionStreamEntry {
            unix_nanos: observation.now_unix_nanos,
            phase: format!("{:?}", self.controller.state.phase),
            mode: format!("{:?}", self.config.mode()),
            focus_kind: observation.focus_kind.map(|kind| format!("{kind:?}")),
            focus_confidence: observation.focus_confidence,
            target_root_pid: observation.target_root_pid,
            active_target_count: observation.active_target_count,
            situation: format!("{:?}", observation.primary_situation),
            situation_confidence: observation.situation.confidence,
            situation_evidence: observation
                .situation
                .evidence
                .iter()
                .take(5)
                .map(|evidence| {
                    format!(
                        "{}={} weight={:.2}",
                        evidence.signal, evidence.value, evidence.weight
                    )
                })
                .collect(),
            situation_blockers: observation
                .situation
                .blockers
                .iter()
                .map(|blocker| format!("{blocker:?}"))
                .collect(),
            protected_tasks_count: observation.protected_tasks.len(),
            candidate_count: self
                .last_plan_result
                .as_ref()
                .map(|plan| plan.evaluations.len())
                .unwrap_or(0),
            top_denied_reason: self
                .last_plan_result
                .as_ref()
                .and_then(top_denied_reason_for_plan),
            planner: self.last_plan_result.as_ref().map(PlanResult::summary),
            dry_run_plan_files: self.last_dry_run_plan_files.clone(),
            score_total: observation.score.total,
            data_quality: data_quality_label(&observation.data_quality),
            data_quality_reason_codes: observation.data_quality.reason_code_strings(),
            decision: decision_label(decision),
            reason,
        }
    }

    pub(super) fn append_decision_log(
        &self,
        entry: &AutotuneDecisionStreamEntry,
    ) -> anyhow::Result<()> {
        let Some(path) = &self.config.decision_log else {
            return Ok(());
        };

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        serde_json::to_writer(&mut file, entry)?;
        file.write_all(b"\n")?;

        Ok(())
    }
}
