//! Replay the live study judgement over independently rebuilt child records.
use std::{collections::BTreeMap, fs::File, os::fd::AsFd};

use super::{
    ResearchIntentV1, ResearchJournal,
    child_ledger::Disposition,
    child_study::{Stage, StudyResult},
    driver::{FailureClass, build_schedule},
    native_context::ProfileConformance,
    report::{AttemptEvidenceV1, ResearchReportV1, evidence_context_consistent},
    report_store::{read_evidence, read_private},
    study_executor::{AttemptsOutcome, judge},
};

/// Only qualified claims are accepted here. Other dispositions need their own independently
/// verifiable interruption or exclusion evidence, and are not promoted by this check.
pub(super) fn verify_qualification(
    dir: &File,
    uid: u32,
    parent: &ResearchIntentV1,
    disposition: &Disposition,
    report: &serde_json::Value,
    raw_verified: bool,
    runtime_matched: bool,
) -> anyhow::Result<()> {
    if disposition.result != Some(StudyResult::Qualified) {
        return Ok(());
    }
    let study = super::child_study::plan(parent)?
        .into_iter()
        .find(|study| study.study_sha256 == disposition.study_sha256)
        .ok_or_else(|| anyhow::anyhow!("qualified study is not planned"))?;
    let intent: ResearchIntentV1 = serde_json::from_slice(&read_private(dir, "intent.json", uid)?)?;
    let report: ResearchReportV1 = serde_json::from_value(report.clone())?;
    let journal = ResearchJournal::open(
        crate::syscall::openat_dir_nofollow(dir.as_fd(), "journal")?,
        uid,
        build_schedule(&intent)?,
    )?;
    let records = journal.verify()?;
    let terminal_failure = records
        .last()
        .and_then(|record| record.outcome.terminal_failure());
    let evidence_dir = File::from(crate::syscall::openat_dir_nofollow(
        dir.as_fd(),
        "evidence",
    )?);
    let evidence: BTreeMap<u32, AttemptEvidenceV1> = read_evidence(&evidence_dir, uid)?;
    let conformance = evidence
        .values()
        .map(|item| {
            item.native_completion
                .as_deref()
                .and_then(|native| native.context.as_ref())
                .map_or(ProfileConformance::Unproven, |context| {
                    context.conformance_in(study.profile, study.stage)
                })
        })
        .collect();
    let (tail_metric, run_fps) = super::metric_plan::run_values(
        evidence
            .values()
            .filter_map(|item| item.native_completion.as_deref()),
    );
    let outcome = AttemptsOutcome {
        planned_attempts: intent.max_attempts,
        source_order: intent.tools.iter().map(|tool| tool.source).collect(),
        attempts_used: u32::try_from(report.attempts_sealed)?,
        completed: u32::try_from(report.attempts_completed)?,
        failures: records
            .iter()
            .filter_map(|record| record.outcome.compatibility_failure().map(str::to_owned))
            .collect(),
        health_halt: terminal_failure == Some(FailureClass::Health),
        terminal_failure,
        context_consistent: evidence_context_consistent(&evidence, intent.treatment.as_ref()),
        conformance,
        raw_verified,
        runtime_matched,
        tail_metric,
        run_fps,
        // The current historical receipts do not independently prove feature activation or an
        // action's objective and guardrails. Never promote those claims from a live boolean.
        stage_evidence: if study.stage == Stage::Init {
            let arms: Vec<&str> = parent
                .prefix_init_arms
                .iter()
                .map(|arm| arm.dir_name())
                .collect();
            read_private(dir, super::init_receipt::INIT_RECEIPT, uid)
                .ok()
                .and_then(|bytes| super::init_receipt::read_init_receipt(&bytes, &arms))
        } else {
            matches!(study.stage, Stage::Feature(_) | Stage::Action(_)).then_some(false)
        },
        activation: None,
        functional: None,
    };
    let replayed = judge(&study, &outcome);
    anyhow::ensure!(
        replayed.result == StudyResult::Qualified && disposition.reason.is_empty(),
        "QualifiedStudyJudgementContradicted: {}",
        replayed.reason
    );
    Ok(())
}
