use serde::{Deserialize, Serialize};

use crate::{
    daemon::health::SystemHealthState,
    diagnosis::{Confidence, StutterCause},
    focus::FocusGroupKind,
    process_tree::TaskClass,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SituationKind {
    Unknown,
    Idle,
    GameFocused,
    GameCpuSchedulerPressure,
    GameGpuBound,
    CompositorPressure,
    CpuPressure,
    IoPressure,
    IrqPressure,
    ThermalOrPowerLimit,
    CompileLoad,
    BrowserFocused,
    BrowserCpuPressure,
    BrowserGpuVideo,
    BrowserIoPressure,
    CompileCpuBound,
    CompileLinkerPressure,
    MediaPlayback,
    Recording,
    VirtualMachineLoad,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SituationClassification {
    pub primary: SituationKind,
    pub secondary: Vec<SituationKind>,
    pub confidence: f32,
    pub evidence: Vec<SituationEvidence>,
    pub blockers: Vec<SituationBlocker>,
    #[serde(default)]
    pub reason_codes: Vec<String>,
}

impl Default for SituationClassification {
    fn default() -> Self {
        Self {
            primary: SituationKind::Unknown,
            secondary: Vec::new(),
            confidence: 0.0,
            evidence: Vec::new(),
            blockers: Vec::new(),
            reason_codes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SituationEvidence {
    pub signal: String,
    pub value: String,
    pub weight: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SituationBlocker {
    LowFocusConfidence,
    LowDataQuality,
    MissingFrameData,
    MissingGpuData,
    MissingIrqData,
    ThermalDegraded,
}

pub fn classify_situation(
    observation: &crate::autotune::observation::AutotuneObservation,
) -> SituationClassification {
    let mut classification = SituationClassification {
        primary: base_situation_from_focus(observation),
        confidence: observation.focus_confidence.clamp(0.0, 1.0),
        ..SituationClassification::default()
    };

    if observation.focus_confidence < crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE {
        classification
            .blockers
            .push(SituationBlocker::LowFocusConfidence);
        classification
            .reason_codes
            .push("low_focus_confidence".to_owned());
    }

    if observation.data_quality.blocks_action() {
        classification
            .blockers
            .push(SituationBlocker::LowDataQuality);
        classification
            .reason_codes
            .push("low_data_quality".to_owned());
    }

    if observation.system_health.state == SystemHealthState::Overheated {
        push_evidence(
            &mut classification,
            "system_health",
            observation.system_health.state.as_str(),
            1.0,
        );
        classification.primary = SituationKind::ThermalOrPowerLimit;
        classification
            .blockers
            .push(SituationBlocker::ThermalDegraded);
        classification
            .reason_codes
            .push("thermal_or_power_limit".to_owned());
        classification.confidence = classification.confidence.max(0.95);
        return dedupe_classification(classification);
    }

    for diagnosis in &observation.recent_diagnoses {
        let Some(kind) = situation_from_diagnosis(observation, diagnosis.cause) else {
            continue;
        };
        let weight = confidence_weight(diagnosis.confidence);
        push_evidence(
            &mut classification,
            "diagnosis",
            format!(
                "{:?}/{:?}/{}",
                diagnosis.cause, diagnosis.anchor_class, diagnosis.anchor_comm
            ),
            weight,
        );
        promote_kind(&mut classification, kind, weight);
    }

    if observation.frame_count == 0 && matches!(classification.primary, SituationKind::GameGpuBound)
    {
        classification
            .blockers
            .push(SituationBlocker::MissingFrameData);
        classification
            .reason_codes
            .push("missing_frame_data".to_owned());
    }

    if classification.evidence.is_empty() {
        add_focus_evidence(observation, &mut classification);
    }

    if classification.primary == SituationKind::Unknown
        && matches!(
            observation.focus_kind,
            Some(FocusGroupKind::Idle | FocusGroupKind::Unknown)
        )
        && !observation.target_present
        && observation.active_target_count == 0
        && observation.scored_samples == 0
    {
        classification.primary = SituationKind::Idle;
        classification.confidence = classification.confidence.max(0.60);
        push_evidence(&mut classification, "activity", "no active target", 0.60);
    }

    dedupe_classification(classification)
}

fn base_situation_from_focus(
    observation: &crate::autotune::observation::AutotuneObservation,
) -> SituationKind {
    match observation.focus_kind {
        Some(FocusGroupKind::Game) => SituationKind::GameFocused,
        Some(FocusGroupKind::Browser) => SituationKind::BrowserFocused,
        Some(FocusGroupKind::Compile) => SituationKind::CompileLoad,
        Some(FocusGroupKind::Media) => SituationKind::MediaPlayback,
        Some(FocusGroupKind::Recording) => SituationKind::Recording,
        Some(FocusGroupKind::VirtualMachine) => SituationKind::VirtualMachineLoad,
        Some(FocusGroupKind::Idle) => SituationKind::Idle,
        Some(FocusGroupKind::Desktop | FocusGroupKind::Unknown) | None => {
            observation.primary_situation
        }
    }
}

fn situation_from_diagnosis(
    observation: &crate::autotune::observation::AutotuneObservation,
    cause: StutterCause,
) -> Option<SituationKind> {
    match cause {
        StutterCause::GameThreadSchedulerDelay => {
            if matches!(observation.focus_kind, Some(FocusGroupKind::Game)) {
                Some(SituationKind::GameCpuSchedulerPressure)
            } else {
                Some(SituationKind::CpuPressure)
            }
        }
        StutterCause::CompositorSchedulerDelay => Some(SituationKind::CompositorPressure),
        StutterCause::GpuBoundCandidate => {
            if matches!(observation.focus_kind, Some(FocusGroupKind::Browser)) {
                Some(SituationKind::BrowserGpuVideo)
            } else {
                Some(SituationKind::GameGpuBound)
            }
        }
        StutterCause::BlockIoCandidate => {
            if matches!(observation.focus_kind, Some(FocusGroupKind::Browser)) {
                Some(SituationKind::BrowserIoPressure)
            } else if observation
                .recent_diagnoses
                .iter()
                .any(|entry| matches!(entry.anchor_class, TaskClass::Linker))
            {
                Some(SituationKind::CompileLinkerPressure)
            } else {
                Some(SituationKind::IoPressure)
            }
        }
        StutterCause::IrqDelayCandidate => Some(SituationKind::IrqPressure),
        StutterCause::CpuPressureCandidate
        | StutterCause::CpuMonopolizationCandidate
        | StutterCause::RuntimeWaitCandidate => Some(cpu_pressure_situation(observation)),
        StutterCause::Unknown => None,
    }
}

fn cpu_pressure_situation(
    observation: &crate::autotune::observation::AutotuneObservation,
) -> SituationKind {
    match observation.focus_kind {
        Some(FocusGroupKind::Game) => SituationKind::GameCpuSchedulerPressure,
        Some(FocusGroupKind::Browser) => SituationKind::BrowserCpuPressure,
        Some(FocusGroupKind::Compile) => {
            if observation
                .recent_diagnoses
                .iter()
                .any(|entry| matches!(entry.anchor_class, TaskClass::Linker))
            {
                SituationKind::CompileLinkerPressure
            } else {
                SituationKind::CompileCpuBound
            }
        }
        Some(FocusGroupKind::Media) => SituationKind::MediaPlayback,
        Some(FocusGroupKind::Recording) => SituationKind::Recording,
        Some(FocusGroupKind::VirtualMachine) => SituationKind::VirtualMachineLoad,
        Some(FocusGroupKind::Idle | FocusGroupKind::Desktop | FocusGroupKind::Unknown) | None => {
            SituationKind::CpuPressure
        }
    }
}

fn confidence_weight(confidence: Confidence) -> f32 {
    match confidence {
        Confidence::High => 0.95,
        Confidence::Medium => 0.70,
        Confidence::Low => 0.40,
    }
}

fn promote_kind(classification: &mut SituationClassification, kind: SituationKind, weight: f32) {
    if weight >= classification.confidence
        || matches!(
            classification.primary,
            SituationKind::Unknown | SituationKind::Idle | SituationKind::GameFocused
        )
    {
        if classification.primary != kind && classification.primary != SituationKind::Unknown {
            classification.secondary.push(classification.primary);
        }
        classification.primary = kind;
        classification.confidence = weight.max(classification.confidence);
    } else if classification.primary != kind {
        classification.secondary.push(kind);
    }
}

fn add_focus_evidence(
    observation: &crate::autotune::observation::AutotuneObservation,
    classification: &mut SituationClassification,
) {
    if let Some(kind) = observation.focus_kind {
        push_evidence(
            classification,
            "focus_kind",
            format!("{kind:?}"),
            observation.focus_confidence.clamp(0.0, 1.0),
        );
    }
}

fn push_evidence(
    classification: &mut SituationClassification,
    signal: impl Into<String>,
    value: impl Into<String>,
    weight: f32,
) {
    classification.evidence.push(SituationEvidence {
        signal: signal.into(),
        value: value.into(),
        weight: weight.clamp(0.0, 1.0),
    });
}

fn dedupe_classification(mut classification: SituationClassification) -> SituationClassification {
    classification
        .secondary
        .retain(|kind| *kind != classification.primary);
    classification
        .secondary
        .sort_by_key(|kind| format!("{kind:?}"));
    classification.secondary.dedup();
    classification.blockers.dedup();
    classification.reason_codes.sort();
    classification.reason_codes.dedup();
    classification
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        autotune::{observation::AutotuneObservation, quality::OnlineDataQuality},
        diagnosis::LiveDiagnosisEntry,
        scorer::StutterScore,
    };

    fn observation(
        focus_kind: FocusGroupKind,
        cause: StutterCause,
        anchor_class: TaskClass,
    ) -> AutotuneObservation {
        AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            active_target_count: 3,
            scored_task_count: 3,
            interval_count: 5,
            scored_samples: 100,
            score: StutterScore {
                total: 500,
                over_1ms: 10,
                over_2ms: 5,
                over_5ms: 1,
                ..StutterScore::default()
            },
            data_quality: OnlineDataQuality::High,
            primary_situation: SituationKind::Unknown,
            focus_kind: Some(focus_kind),
            focus_confidence: 0.92,
            focus_roots: vec![1234],
            recent_diagnoses: vec![LiveDiagnosisEntry {
                elapsed_ms: 100,
                cause,
                confidence: Confidence::High,
                anchor_class,
                anchor_comm: "target".to_owned(),
                evidence: vec!["synthetic evidence".to_owned()],
            }],
            frame_count: 100,
            frame_p99_ms: 30.0,
            frame_max_ms: 50.0,
            ..AutotuneObservation::default()
        }
    }

    #[test]
    fn game_focus_with_scheduler_spikes_classifies_game_cpu_scheduler_pressure() {
        let observation = observation(
            FocusGroupKind::Game,
            StutterCause::GameThreadSchedulerDelay,
            TaskClass::GameRenderThread,
        );

        let classification = classify_situation(&observation);

        assert_eq!(
            classification.primary,
            SituationKind::GameCpuSchedulerPressure
        );
        assert!(classification.confidence >= 0.90);
    }

    #[test]
    fn game_focus_with_bad_frames_and_gpu_evidence_classifies_game_gpu_bound() {
        let observation = observation(
            FocusGroupKind::Game,
            StutterCause::GpuBoundCandidate,
            TaskClass::GameRenderThread,
        );

        let classification = classify_situation(&observation);

        assert_eq!(classification.primary, SituationKind::GameGpuBound);
    }

    #[test]
    fn browser_focus_with_cpu_pressure_classifies_browser_cpu_pressure() {
        let observation = observation(
            FocusGroupKind::Browser,
            StutterCause::CpuPressureCandidate,
            TaskClass::BrowserForeground,
        );

        let classification = classify_situation(&observation);

        assert_eq!(classification.primary, SituationKind::BrowserCpuPressure);
    }

    #[test]
    fn compile_focus_with_linker_comm_classifies_compile_linker_pressure() {
        let observation = observation(
            FocusGroupKind::Compile,
            StutterCause::CpuPressureCandidate,
            TaskClass::Linker,
        );

        let classification = classify_situation(&observation);

        assert_eq!(classification.primary, SituationKind::CompileLinkerPressure);
    }

    #[test]
    fn low_quality_preserves_situation_but_marks_blocker() {
        let mut observation = observation(
            FocusGroupKind::Browser,
            StutterCause::CpuPressureCandidate,
            TaskClass::BrowserForeground,
        );
        observation.data_quality = OnlineDataQuality::Low {
            reasons: vec!["drop_counter_total".to_owned()],
        };

        let classification = classify_situation(&observation);

        assert_eq!(classification.primary, SituationKind::BrowserCpuPressure);
        assert!(
            classification
                .blockers
                .contains(&SituationBlocker::LowDataQuality)
        );
    }

    #[test]
    fn unknown_focus_does_not_invent_candidate_situation() {
        let mut observation = AutotuneObservation::default();
        observation.focus_kind = Some(FocusGroupKind::Unknown);
        observation.focus_confidence = 0.2;

        let classification = classify_situation(&observation);

        assert_eq!(classification.primary, SituationKind::Idle);
    }
}
