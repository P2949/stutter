//! Replay the live study judgement over independently rebuilt child records.
use std::{collections::BTreeMap, fs::File, os::fd::AsFd};

use super::{
    ResearchIntentV1, ResearchJournal,
    child_ledger::Disposition,
    child_study::{Stage, StudyResult},
    driver::{FailureClass, build_schedule},
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
    let qualified = disposition.result == Some(StudyResult::Qualified);
    // A feature study that was not qualified is replayed too: its reason must follow from its sealed
    // records and its recorded per-attempt observation, never from a live boolean.
    let feature_unqualified = disposition.result == Some(StudyResult::Unqualified)
        && matches!(disposition.stage, Stage::Feature(_));
    // A justified abstention is replayed too: the study ran, the controller never applied the action, and its
    // own recorded abstention (with no receipts) must match the sealed attempt count.
    let abstained = disposition.result == Some(StudyResult::Unqualified)
        && matches!(disposition.stage, Stage::Action(_))
        && disposition.reason == "ActionNotIndicated";
    if !qualified && !feature_unqualified && !abstained {
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
    let applied = read_private(dir, super::init_receipt::SETTINGS_RECEIPT, uid)
        .ok()
        .and_then(|bytes| super::init_receipt::read_settings_receipt(&bytes));
    // Every study this build verifies records what its profile left (an older one is not replayed).
    let conformance =
        super::settings_binding::conformances(&evidence, applied.as_deref(), &intent, &study, true);
    let (tail_metric, run_fps) = super::metric_plan::run_values(
        evidence
            .values()
            .filter_map(|item| item.native_completion.as_deref()),
    );
    let (activation, functional) = feature_replay(dir, uid, &intent, &study, &evidence);
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
        comparison_valid: super::study_executor::comparison_valid(study.stage, &report.arms),
        action_metric_valid: matches!(study.stage, Stage::Action(_))
            && action_metric_valid(dir, uid, &intent, &evidence),
        action_not_indicated: matches!(study.stage, Stage::Action(action)
            if abstention_replay(dir, uid, u32::try_from(report.attempts_completed)?)
                && planner_observation_valid(dir, uid, action, &evidence)),
        stage_evidence: if study.stage == Stage::Init {
            let arms: Vec<&str> = parent
                .prefix_init_arms
                .iter()
                .map(|arm| arm.dir_name())
                .collect();
            read_private(dir, super::init_receipt::INIT_RECEIPT, uid)
                .ok()
                .and_then(|bytes| super::init_receipt::read_init_receipt(&bytes, &arms))
        } else if let Stage::Action(action) = study.stage {
            // Rebuilt from the sealed receipts and the schedule, the way the live run derived it from its gate.
            Some(action_replay(
                dir, uid, parent.uid, action, &intent, &evidence,
            ))
        } else {
            matches!(study.stage, Stage::Feature(_)).then_some(false)
        },
        activation,
        functional,
    };
    let replayed = judge(&study, &outcome);
    if qualified {
        anyhow::ensure!(
            replayed.result == StudyResult::Qualified && disposition.reason.is_empty(),
            "QualifiedStudyJudgementContradicted: {}",
            replayed.reason
        );
    } else {
        anyhow::ensure!(
            replayed.result == StudyResult::Unqualified && replayed.reason == disposition.reason,
            "UnqualifiedFeatureJudgementContradicted: {} vs {}",
            replayed.reason,
            disposition.reason
        );
    }
    Ok(())
}

/// A feature study's activation and function, rebuilt without the live run: the recorded per-attempt
/// observation must name exactly the completed slots with the treatments the intent's own schedule gives
/// them, and the game's report comes from the sealed native contexts. Anything missing or inconsistent is
/// `(None, None)`, which the judgement reads as unproven.
fn feature_replay(
    dir: &File,
    uid: u32,
    intent: &ResearchIntentV1,
    study: &super::child_study::ChildStudy,
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
) -> (Option<bool>, Option<super::functional_probe::Functional>) {
    use super::Treatment;
    let Stage::Feature(factor) = study.stage else {
        return (None, None);
    };
    let replayed = (|| {
        let observed = super::init_receipt::read_feature_receipt(
            &read_private(dir, super::init_receipt::FEATURE_RECEIPT, uid).ok()?,
        )?;
        let schedule = super::treatment::treatments_of(intent).ok()??;
        // The same slots as the completed evidence, each with its scheduled treatment.
        if observed.keys().ne(evidence.keys())
            || observed
                .iter()
                .any(|(slot, (treatment, _))| schedule.get(*slot as usize) != Some(treatment))
        {
            return None;
        }
        let both = [Treatment::Off, Treatment::On]
            .iter()
            .all(|wanted| observed.values().any(|(t, _)| t == wanted));
        let activation = both && observed.values().all(|(_, reached)| *reached);
        let attempts: Vec<_> = evidence
            .iter()
            .map(|(slot, item)| {
                (
                    observed[slot].0,
                    item.native_completion
                        .as_deref()
                        .and_then(|native| native.context.as_ref()),
                )
            })
            .collect();
        Some((
            Some(activation),
            Some(super::functional_probe::probe(factor, &attempts)),
        ))
    })();
    replayed.unwrap_or((None, None))
}

/// Whether an action study's sealed receipts show an applied and restored action on every attempt the
/// schedule treated On, each attributed to the very process that attempt recorded. This is what the live gate
/// proved when it said the action had evidence; anything missing, malformed or unattributable is `false`.
pub(super) fn action_replay(
    dir: &File,
    uid: u32,
    owner: u32,
    action: super::ResearchAction,
    intent: &ResearchIntentV1,
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
) -> bool {
    (|| -> Option<bool> {
        let receipts: serde_json::Value =
            serde_json::from_slice(&read_private(dir, "action-receipts.json", uid).ok()?).ok()?;
        super::child_verify_rules::receipts_valid(&receipts, action, owner).ok()?;
        let treatments = super::treatment::treatments_of(intent).ok()??;
        let processes = super::child_verify_rules::processes_of(evidence);
        // Every receipt's whole timeline lies inside the very attempt it is attributed to.
        for item in receipts.as_array()? {
            let process = processes.iter().find(|p| {
                item["pid"].as_u64() == Some(u64::from(p.pid.as_u32()))
                    && item["start_ticks"].as_u64() == Some(p.start_ticks)
            })?;
            let (permitted, settled) = (
                item["permitted_unix"].as_u64()?,
                item["settled_unix"].as_u64()?,
            );
            if permitted < process.started_unix || settled > process.completed_unix {
                return Some(false);
            }
        }
        let coverage =
            super::action_coverage::attribute(&receipts, &processes, &treatments).ok()?;
        Some(coverage.applied_on_every_treated_attempt())
    })()
    .unwrap_or(false)
}

/// Whether the action study's own durable abstention record names exactly the completed attempts, says nothing
/// was applied, and sits beside no action receipts (an applied action would have left some).
/// Whether the planner's own sealed observation supports abstaining from this action over these completed attempts.
pub(super) fn planner_observation_valid(
    dir: &File,
    uid: u32,
    action: super::ResearchAction,
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
) -> bool {
    read_private(dir, super::planner_observation::FILE, uid).is_ok_and(|bytes| {
        super::planner_observation::verify(
            &bytes,
            action,
            &super::child_verify_rules::processes_of(evidence),
        )
        .is_ok()
    })
}

fn abstention_replay(dir: &File, uid: u32, completed: u32) -> bool {
    let recorded = read_private(dir, super::action_coverage::ABSTENTION_RECORD, uid)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    recorded.is_some_and(|value| {
        value["completed"].as_u64() == Some(u64::from(completed)) && value["applied"] == 0
    }) && matches!(
        crate::syscall::openat_nofollow(dir.as_fd(), "action-receipts.json"),
        Err(ref error) if error.kind() == std::io::ErrorKind::NotFound
    )
}

/// Whether the action study's sealed paired-latency record is a valid measurement for this schedule and these
/// attempts. Live outcome and replay both use this one function; absent or unreadable is simply not valid.
pub(super) fn action_metric_valid(
    dir: &File,
    uid: u32,
    intent: &ResearchIntentV1,
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
) -> bool {
    (|| {
        let bytes = read_private(dir, super::action_latency::FILE, uid).ok()?;
        let treatments = super::treatment::treatments_of(intent).ok()??;
        let processes = super::child_verify_rules::processes_of(evidence);
        super::action_latency::verify(&bytes, &treatments, &processes).ok()
    })()
    .is_some()
}
