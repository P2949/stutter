//! Status the LLM reads instead of `PLAN.md`. Stable JSON; never exposes raw command lines.
use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

use super::{
    graph::{Graph, issue_gates},
    manifest::CampaignManifest,
    model::{
        AttemptOutcome, CampaignMode, CampaignPhase, CampaignState, FailureClass, RepositoryAction,
        TaskStatus,
    },
    repository::repository_actions,
};

#[derive(Debug, Serialize)]
pub struct FailedTask {
    pub task: String,
    pub status: TaskStatus,
    pub stage: String,
    pub attempt: u32,
    pub artifacts: Option<String>,
    pub error_summary: String,
}

#[derive(Debug, Serialize)]
pub struct BlockedTask {
    pub task: String,
    pub status: TaskStatus,
    pub class: FailureClass,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct KernelProgress {
    pub required: usize,
    pub observed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub schema_version: u32,
    pub campaign_id: String,
    pub generation: u32,
    pub mode: CampaignMode,
    pub evidence_base: String,
    pub manifest_sha256: String,
    pub overall: String,
    pub phase: Option<CampaignPhase>,
    pub next_tasks: Vec<String>,
    pub running_tasks: Vec<String>,
    pub failed_tasks: Vec<FailedTask>,
    pub blocked: Vec<BlockedTask>,
    pub kernel_progress: KernelProgress,
    pub corpus: Value,
    pub issues: BTreeMap<String, Value>,
    pub policy: Value,
    pub release: Value,
    pub repository_actions: Vec<RepositoryAction>,
}

/// Observed distinct kernel releases are frozen into state as a comma-separated list by the
/// kernel stage; status only reports them.
fn observed_kernels(state: &CampaignState) -> Vec<String> {
    state
        .frozen
        .get("kernel.observed")
        .map(|list| {
            list.split(',')
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn build(state: &CampaignState, graph: &Graph, manifest: &CampaignManifest) -> StatusReport {
    let effective = |id: &str| graph.get(id).map(|spec| state.effective_status(spec));
    let mut failed_tasks = Vec::new();
    let mut blocked = Vec::new();
    let mut running_tasks = Vec::new();
    for id in graph.topological() {
        let Some(task) = state.tasks.get(id) else {
            continue;
        };
        let status = effective(id).unwrap_or(task.status);
        match status {
            TaskStatus::Running => running_tasks.push(id.to_owned()),
            TaskStatus::Failed => {
                if let Some(attempt) = task.attempts.last()
                    && let AttemptOutcome::Failed { stage, summary, .. } = &attempt.outcome
                {
                    failed_tasks.push(FailedTask {
                        task: id.to_owned(),
                        status,
                        stage: stage.clone(),
                        attempt: attempt.number,
                        artifacts: attempt.artifacts.clone(),
                        error_summary: summary.clone(),
                    });
                }
            }
            TaskStatus::BlockedResource | TaskStatus::BlockedPrivilege => {
                if let Some(reason) = &task.block {
                    blocked.push(BlockedTask {
                        task: id.to_owned(),
                        status,
                        class: reason.class,
                        detail: reason.detail.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    let software_defect = failed_tasks.iter().any(|failed| {
        state.tasks.get(&failed.task).is_some_and(|task| {
            matches!(
                task.attempts.last().map(|a| &a.outcome),
                Some(AttemptOutcome::Failed {
                    class: FailureClass::SoftwareDefect,
                    ..
                })
            )
        })
    });
    let waiting = state
        .tasks
        .values()
        .any(|task| task.status == TaskStatus::WaitingForReboot);
    let required = graph
        .specs()
        .filter(|spec| {
            state.tasks.get(spec.id).map(|t| t.status) != Some(TaskStatus::SkippedByPolicy)
        })
        .collect::<Vec<_>>();
    let all_passed = required
        .iter()
        .all(|spec| state.effective_status(spec).satisfies_dependents());
    let overall = match state.mode {
        CampaignMode::Invalidated => "invalidated",
        _ if state.mode == CampaignMode::Complete && all_passed => "complete",
        _ if all_passed => "complete",
        _ if software_defect => "failure_requires_source_change",
        _ if waiting => "waiting_for_reboot",
        _ if blocked
            .iter()
            .any(|b| b.class == FailureClass::BlockedPrivilege) =>
        {
            "blocked_privilege"
        }
        _ if !blocked.is_empty() => "blocked_resource",
        _ => "running",
    }
    .to_owned();
    let phase = required
        .iter()
        .filter(|spec| !state.effective_status(spec).satisfies_dependents())
        .map(|spec| spec.phase)
        .min();
    let issues = issue_gates()
        .into_iter()
        .map(|gate| {
            let tasks = gate
                .tasks
                .iter()
                .map(|task| {
                    (
                        task.clone(),
                        effective(task).map_or("Unknown".to_owned(), |s| format!("{s:?}")),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let ready = tasks.values().all(|status| status == "Passed");
            (
                gate.issue.to_string(),
                json!({"title": gate.title, "ready": ready, "tasks": tasks}),
            )
        })
        .collect();
    StatusReport {
        schema_version: 1,
        campaign_id: state.campaign_id.clone(),
        generation: state.generation,
        mode: state.mode,
        evidence_base: state.evidence_base.clone(),
        manifest_sha256: state.manifest_sha256.clone(),
        overall,
        phase,
        next_tasks: state.ready(graph).into_iter().map(str::to_owned).collect(),
        running_tasks,
        failed_tasks,
        blocked,
        kernel_progress: KernelProgress {
            required: manifest.kernels.required_distinct,
            observed: observed_kernels(state),
        },
        corpus: state
            .frozen
            .get("corpus.summary")
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_else(|| json!({})),
        issues,
        policy: json!({"decisions": state.decisions}),
        release: json!({
            "version": manifest.release.version,
            "tag": manifest.release.tag,
            "auto_tag": manifest.release.auto_tag,
            "candidate": state.release_candidate,
        }),
        repository_actions: repository_actions(state, graph),
    }
}

pub fn render_human(report: &StatusReport) -> String {
    let mut text = format!(
        "campaign {} generation {} mode {:?}\noverall: {}\nevidence base: {}\n",
        report.campaign_id, report.generation, report.mode, report.overall, report.evidence_base
    );
    if let Some(phase) = report.phase {
        text.push_str(&format!("phase: {phase:?}\n"));
    }
    text.push_str(&format!("next: {}\n", report.next_tasks.join(", ")));
    for failed in &report.failed_tasks {
        text.push_str(&format!(
            "FAILED {} attempt {} at {}: {}\n",
            failed.task, failed.attempt, failed.stage, failed.error_summary
        ));
    }
    for block in &report.blocked {
        text.push_str(&format!(
            "BLOCKED {} ({:?}): {}\n",
            block.task, block.class, block.detail
        ));
    }
    text.push_str(&format!(
        "kernels: {}/{} observed\n",
        report.kernel_progress.observed.len(),
        report.kernel_progress.required
    ));
    for action in report.repository_actions.iter().filter(|a| a.ready) {
        text.push_str(&format!("ACTION {} {:?}\n", action.kind, action.issue));
    }
    text
}
