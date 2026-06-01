use std::path::Path;

use super::models::*;
use crate::{
    diagnosis::{Confidence, StutterCause},
    irq_inspect::{IrqLine, classify_irq_device},
    report::{self, DataQualityLevel, ReportAnalysisJson},
};

pub fn build_advisor_report(run: &Path, profiles: Option<&Path>) -> anyhow::Result<AdvisorReport> {
    let analysis = report::build_report_analysis(run, 10, 5, None)?;
    Ok(build_advisor_report_from_analysis(run, profiles, &analysis))
}

pub fn build_advisor_report_from_analysis(
    run: &Path,
    profiles: Option<&Path>,
    analysis: &ReportAnalysisJson,
) -> AdvisorReport {
    let causes = causes_from_analysis(analysis);
    let cause_evidence = cause_evidence_from_analysis(analysis);
    let irq_inventory = analysis.session.core.metadata.irq_lines.as_slice();
    build_advisor_report_from_evidence(AdvisorEvidenceInput {
        run,
        data_quality: analysis.data_quality.level,
        causes: &causes,
        cause_evidence: &cause_evidence,
        profiles,
        signal_availability: AdvisorSignalAvailability {
            has_hwmon: analysis.session.config.hwmon
                || analysis.artifacts_summary.gpu_sample_count > 0,
            has_irq: analysis.session.config.irq_latency
                || analysis.artifacts_summary.irq_event_count > 0,
            has_block_io: analysis.session.config.block_io
                || analysis.artifacts_summary.block_io_event_count > 0,
        },
        tree_pid: analysis.session.config.tree_roots.first().copied(),
        irq_inventory,
    })
}
pub(crate) fn build_advisor_report_from_evidence(input: AdvisorEvidenceInput<'_>) -> AdvisorReport {
    let run = input.run;
    let data_quality = input.data_quality;
    let causes = input.causes;
    let cause_evidence = input.cause_evidence;
    let profiles = input.profiles;
    let has_hwmon = input.signal_availability.has_hwmon;
    let has_irq = input.signal_availability.has_irq;
    let has_block_io = input.signal_availability.has_block_io;
    let tree_pid = input.tree_pid;
    let irq_inventory = input.irq_inventory;
    let mut warnings = Vec::new();
    let mut recommendations = Vec::new();

    if data_quality == DataQualityLevel::Low {
        recommendations.push(AdvisorRecommendation {
            title: "Collect more data".to_owned(),
            rationale:
                "Data quality is low, so advisor output is only a candidate signal and not proof."
                    .to_owned(),
            confidence: Confidence::Medium,
            suggested_commands: vec![
                "stutter bench --duration 180 --scenario <name> --role baseline".to_owned(),
            ],
            safety_note: "Observe only; do not auto-apply tuning from this run.".to_owned(),
        });
        return AdvisorReport {
            schema_version: 1,
            run: run.to_path_buf(),
            data_quality,
            verdict: AdvisorVerdict::CollectMoreData,
            recommendations,
            warnings,
        };
    }

    let has_scheduler = causes.iter().any(|cause| {
        matches!(
            cause,
            StutterCause::CompositorSchedulerDelay | StutterCause::GameThreadSchedulerDelay
        )
    });
    let has_gpu = causes.contains(&StutterCause::GpuBoundCandidate);
    let has_irq_candidate = causes.contains(&StutterCause::IrqDelayCandidate);
    let has_block_io_candidate = causes.contains(&StutterCause::BlockIoCandidate);

    if has_gpu {
        let gpu_note = gpu_specific_note(cause_evidence);
        recommendations.push(AdvisorRecommendation {
            title: "Investigate non-CPU bottleneck candidate".to_owned(),
            rationale: rationale_with_evidence(
                gpu_note.as_deref().unwrap_or(
                    "GPU-bound evidence is a candidate, not proof; CPU affinity may not fix this.",
                ),
                evidence_note_for(cause_evidence, StutterCause::GpuBoundCandidate),
            ),
            confidence: Confidence::Medium,
            suggested_commands: if has_hwmon {
                vec![
                    "stutter report --analysis-json <run-dir>".to_owned(),
                    "stutter display-path compare --baseline <baseline-run> --test <test-run>"
                        .to_owned(),
                ]
            } else {
                vec![
                    "stutter record --hwmon --drm-fence-latency --duration 180 --run-name gpu-check"
                        .to_owned(),
                ]
            },
            safety_note: "Observe only; do not auto-apply CPU affinity for a GPU-bound candidate."
                .to_owned(),
        });
        warnings.push("CPU affinity may not help a GPU-bound candidate.".to_owned());
    }

    if has_irq_candidate {
        let irq_specific = irq_specific_rationale(cause_evidence, irq_inventory);
        recommendations.push(AdvisorRecommendation {
            title: irq_specific
                .as_ref()
                .map(|_| "Inspect specific IRQ affinity candidate")
                .unwrap_or("Confirm IRQ latency candidate")
                .to_owned(),
            rationale: rationale_with_evidence(
                irq_specific.as_deref().unwrap_or(
                    "IRQ overlap is a candidate signal, not proof; collect explicit IRQ data before changing anything.",
                ),
                evidence_note_for(cause_evidence, StutterCause::IrqDelayCandidate),
            ),
            confidence: Confidence::Medium,
            suggested_commands: if has_irq {
                if let Some(irq) = first_irq_number_from_evidence(cause_evidence) {
                    vec![
                        "stutter report --analysis-json <run-dir>".to_owned(),
                        format!("stutter irq inspect --top 20 | grep '^{irq}:'"),
                    ]
                } else {
                    vec!["stutter report --analysis-json <run-dir>".to_owned()]
                }
            } else {
                vec!["stutter record --irq-latency --irq <IRQ> --duration 180 --run-name irq-check".to_owned()]
            },
            safety_note: "Observe only; inspect IRQ affinity before changing it.".to_owned(),
        });
        warnings.push(
            "Advisor does not auto-suggest changing IRQ affinity; inspect the specific IRQ first."
                .to_owned(),
        );
    }

    if has_block_io_candidate {
        recommendations.push(AdvisorRecommendation {
            title: "Check storage activity candidate".to_owned(),
            rationale: rationale_with_evidence(
                "Block I/O overlap is a candidate, not proof; storage pressure should be confirmed before CPU tuning.",
                evidence_note_for(cause_evidence, StutterCause::BlockIoCandidate),
            ),
            confidence: Confidence::Medium,
            suggested_commands: if has_block_io {
                vec!["stutter report --analysis-json <run-dir>".to_owned()]
            } else {
                vec!["stutter record --block-io --duration 180 --run-name io-check".to_owned()]
            },
            safety_note: "Observe only; do not tune CPU affinity first for a block I/O candidate.".to_owned(),
        });
    }

    if has_scheduler {
        let profiles_arg = profiles
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<profiles.toml>".to_owned());
        let pid_arg = tree_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "<PID>".to_owned());
        recommendations.push(AdvisorRecommendation {
            title: "Try profile tuning experiment".to_owned(),
            rationale: rationale_with_evidence(
                "Scheduler-delay candidates suggest a profile tuning experiment may be useful, but this is not proof of root cause.",
                scheduler_evidence_note(cause_evidence),
            ),
            confidence: Confidence::Medium,
            suggested_commands: vec![format!(
                "stutter tune --tree-pid {pid_arg} --profiles {profiles_arg} --runs 5 --baseline-profile baseline-online"
            )],
            safety_note: "Suggested experiment only; do not auto-apply the result.".to_owned(),
        });
        if profiles.is_none() {
            warnings.push(
                "No profiles file was provided; create one from examples/profiles before tuning."
                    .to_owned(),
            );
        }
    }

    let verdict = if has_gpu || has_irq_candidate || has_block_io_candidate {
        AdvisorVerdict::InvestigateNonCpuBottleneck
    } else if has_scheduler {
        AdvisorVerdict::TryProfileTuning
    } else {
        if recommendations.is_empty() {
            recommendations.push(AdvisorRecommendation {
                title: "Collect more comparable data".to_owned(),
                rationale:
                    "No strong candidate stood out; this is not proof that no bottleneck exists."
                        .to_owned(),
                confidence: Confidence::Low,
                suggested_commands: vec![
                    "stutter bench --duration 180 --scenario <name> --role baseline".to_owned(),
                ],
                safety_note: "Observe only; do not auto-apply tuning from this run.".to_owned(),
            });
        }
        AdvisorVerdict::CollectMoreData
    };

    AdvisorReport {
        schema_version: 1,
        run: run.to_path_buf(),
        data_quality,
        verdict,
        recommendations,
        warnings,
    }
}

fn rationale_with_evidence(base: &str, evidence_note: Option<String>) -> String {
    match evidence_note {
        Some(note) => format!("{base} Evidence: {note}"),
        None => base.to_owned(),
    }
}

fn scheduler_evidence_note(cause_evidence: &[AdvisorCauseEvidence]) -> Option<String> {
    evidence_note_for(cause_evidence, StutterCause::GameThreadSchedulerDelay)
        .or_else(|| evidence_note_for(cause_evidence, StutterCause::CompositorSchedulerDelay))
}

fn first_irq_number_from_evidence(cause_evidence: &[AdvisorCauseEvidence]) -> Option<u32> {
    let evidence = evidence_note_for(cause_evidence, StutterCause::IrqDelayCandidate)?;

    let marker = "IRQ ";
    let start = evidence.find(marker)? + marker.len();
    let tail = &evidence[start..];
    let digits = tail
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();

    digits.parse().ok()
}

fn irq_line_for_number(irq_inventory: &[IrqLine], irq: u32) -> Option<&IrqLine> {
    let irq = irq.to_string();
    irq_inventory.iter().find(|line| line.irq == irq)
}

fn irq_specific_rationale(
    cause_evidence: &[AdvisorCauseEvidence],
    irq_inventory: &[IrqLine],
) -> Option<String> {
    let irq = first_irq_number_from_evidence(cause_evidence)?;
    let line = irq_line_for_number(irq_inventory, irq)?;
    let class = classify_irq_device(line);

    Some(format!(
        "IRQ {irq} ({}, class={class:?}) overlapped with the spike. Inspect IRQ affinity for this specific interrupt before changing CPU tuning.",
        line.name
    ))
}

fn gpu_specific_note(cause_evidence: &[AdvisorCauseEvidence]) -> Option<String> {
    let messages = cause_evidence
        .iter()
        .find(|entry| entry.cause == StutterCause::GpuBoundCandidate)?
        .messages
        .iter()
        .filter(|message| {
            message.contains("power limit")
                || message.contains("DRM fence wait")
                || message.contains("GPU busy")
        })
        .take(3)
        .cloned()
        .collect::<Vec<_>>();

    (!messages.is_empty()).then(|| messages.join("; "))
}

fn evidence_note_for(
    cause_evidence: &[AdvisorCauseEvidence],
    cause: StutterCause,
) -> Option<String> {
    cause_evidence
        .iter()
        .find(|entry| entry.cause == cause)
        .and_then(|entry| evidence_note_from_messages(&entry.messages))
}

fn evidence_note_from_messages(messages: &[String]) -> Option<String> {
    let selected = messages
        .iter()
        .filter(|message| !message.trim().is_empty())
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    (!selected.is_empty()).then(|| selected.join("; "))
}

fn cause_evidence_from_analysis(analysis: &ReportAnalysisJson) -> Vec<AdvisorCauseEvidence> {
    let mut summaries = Vec::new();
    for cluster in analysis.cluster_analysis.clusters.iter().take(10) {
        if let Some(diagnosis) = &cluster.diagnosis {
            push_diagnosis_evidence(&mut summaries, diagnosis);
        }
    }
    for frame in analysis.frame_diagnoses.iter().take(10) {
        push_diagnosis_evidence(&mut summaries, &frame.diagnosis);
    }
    summaries
}

fn push_diagnosis_evidence(
    summaries: &mut Vec<AdvisorCauseEvidence>,
    diagnosis: &crate::diagnosis::Diagnosis,
) {
    for candidate in diagnosis.primary.iter().chain(diagnosis.candidates.iter()) {
        let messages = candidate
            .evidence
            .iter()
            .map(|evidence| evidence.message.clone())
            .filter(|message| !message.trim().is_empty())
            .take(3)
            .collect::<Vec<_>>();
        if messages.is_empty() {
            continue;
        }
        if let Some(existing) = summaries
            .iter_mut()
            .find(|entry| entry.cause == candidate.cause)
        {
            for message in messages {
                if existing.messages.len() >= 3 {
                    break;
                }
                if !existing.messages.contains(&message) {
                    existing.messages.push(message);
                }
            }
        } else {
            summaries.push(AdvisorCauseEvidence {
                cause: candidate.cause,
                messages,
            });
        }
    }
}
fn causes_from_analysis(analysis: &ReportAnalysisJson) -> Vec<StutterCause> {
    let mut causes = Vec::new();
    for cluster in analysis.cluster_analysis.clusters.iter().take(10) {
        if let Some(diagnosis) = &cluster.diagnosis {
            if let Some(primary) = &diagnosis.primary {
                causes.push(primary.cause);
            } else {
                causes.push(diagnosis.cause);
            }
            causes.extend(diagnosis.secondary_causes.iter().copied());
        }
    }
    for frame in analysis.frame_diagnoses.iter().take(10) {
        if let Some(primary) = &frame.diagnosis.primary {
            causes.push(primary.cause);
        } else {
            causes.push(frame.diagnosis.cause);
        }
    }
    causes
}
