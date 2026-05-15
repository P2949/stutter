use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    quality::OnlineDataQuality,
    situation::{SituationClassification, SituationKind, classify_situation},
};
use crate::{
    daemon::{capabilities::DaemonCapabilities, health::SystemHealthSnapshot},
    diagnosis::LiveDiagnosisEntry,
    focus::FocusGroupKind,
    process_tree::TaskClass,
    scorer::StutterScore,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadIdentity {
    pub root_pid: u32,
    pub process_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub cgroup_path: Option<String>,
    pub focus_kind: Option<FocusGroupKind>,
    pub class_distribution: BTreeMap<String, usize>,
    pub stable_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtectedTask {
    pub tid: u32,
    pub process_pid: u32,
    pub comm: String,
    pub class: TaskClass,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveTaskSnapshot {
    pub tid: u32,
    pub process_pid: u32,
    pub comm: String,
    pub class: TaskClass,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
    pub cgroup_path: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveConfigSnapshot {
    pub affinity: Option<String>,
    pub nice: Option<String>,
    pub uclamp: Option<String>,
    pub cgroup: Option<String>,
    pub irq: Option<String>,
    pub cpu_power: Option<String>,
    pub gpu_power: Option<String>,
    pub vm: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutotuneObservation {
    pub now_unix_nanos: u128,
    pub elapsed_ms: u64,

    pub target_present: bool,
    pub target_root_pid: Option<u32>,
    pub active_target_count: usize,
    pub scored_task_count: usize,

    pub interval_count: usize,
    pub scored_samples: u64,

    pub score: StutterScore,
    pub data_quality: OnlineDataQuality,

    pub primary_situation: SituationKind,
    pub situation: SituationClassification,
    pub focus_kind: Option<crate::focus::FocusGroupKind>,
    pub focus_confidence: f32,
    pub focus_roots: Vec<u32>,
    pub focus_reasons: Vec<String>,
    pub recent_diagnoses: Vec<LiveDiagnosisEntry>,
    pub system_health: SystemHealthSnapshot,
    pub capabilities: DaemonCapabilities,
    pub topology_signature: Option<String>,
    pub workload_identity: Option<WorkloadIdentity>,
    pub active_tasks: Vec<ActiveTaskSnapshot>,
    pub protected_tasks: Vec<ProtectedTask>,
    pub active_config_snapshot: Option<ActiveConfigSnapshot>,

    pub frame_count: usize,
    pub frame_p99_ms: f64,
    pub frame_max_ms: f64,

    pub drop_counter_total: u64,
}

impl Default for AutotuneObservation {
    fn default() -> Self {
        Self {
            now_unix_nanos: 0,
            elapsed_ms: 0,
            target_present: false,
            target_root_pid: None,
            active_target_count: 0,
            scored_task_count: 0,
            interval_count: 0,
            scored_samples: 0,
            score: StutterScore::default(),
            data_quality: OnlineDataQuality::default(),
            primary_situation: SituationKind::Unknown,
            situation: SituationClassification::default(),
            focus_kind: None,
            focus_confidence: 0.0,
            focus_roots: Vec::new(),
            focus_reasons: Vec::new(),
            recent_diagnoses: Vec::new(),
            system_health: SystemHealthSnapshot::default(),
            capabilities: DaemonCapabilities::default(),
            topology_signature: None,
            workload_identity: None,
            active_tasks: Vec::new(),
            protected_tasks: Vec::new(),
            active_config_snapshot: None,
            frame_count: 0,
            frame_p99_ms: 0.0,
            frame_max_ms: 0.0,
            drop_counter_total: 0,
        }
    }
}

impl AutotuneObservation {
    pub fn apply_focus_context(&mut self, focus: Option<&crate::focus::ResolvedFocus>) {
        if let Some(focus) = focus {
            self.focus_kind = Some(focus.group.kind);
            self.focus_confidence = focus.group.confidence;
            self.focus_roots = focus.group.root_pids.clone();
            self.focus_reasons = focus.group.reasons.clone();
            self.primary_situation = focus.situation;
            self.refresh_situation_classification();
        } else {
            self.focus_kind = None;
            self.focus_confidence = 0.0;
            self.focus_roots.clear();
            self.focus_reasons.clear();
            self.primary_situation = SituationKind::Unknown;
            self.refresh_situation_classification();
        }
    }

    pub fn refresh_situation_classification(&mut self) {
        self.situation = classify_situation(self);
        self.primary_situation = self.situation.primary;
    }

    pub fn focus_is_idle_or_unknown(&self) -> bool {
        matches!(
            self.focus_kind,
            None | Some(crate::focus::FocusGroupKind::Idle)
                | Some(crate::focus::FocusGroupKind::Unknown)
        ) || matches!(
            self.primary_situation,
            SituationKind::Idle | SituationKind::Unknown
        )
    }

    pub fn focus_has_critical_realtime_warning(&self) -> bool {
        self.focus_reasons.iter().any(|reason| {
            let lower = reason.to_ascii_lowercase();
            lower.contains("critical realtime")
                || lower.contains("critical realtime/input")
                || lower.contains("audio realtime")
                || lower.contains("input process")
        })
    }
}

pub fn is_protected_task_class(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::AudioRealtime
            | TaskClass::Input
            | TaskClass::Compositor
            | TaskClass::GameScope
            | TaskClass::Media
            | TaskClass::Recorder
            | TaskClass::KernelThread
            | TaskClass::IrqThread
            | TaskClass::Service
            | TaskClass::NetworkDaemon
            | TaskClass::StorageDaemon
    )
}

pub fn protected_task_reason(class: TaskClass) -> &'static str {
    match class {
        TaskClass::AudioRealtime => "audio realtime task",
        TaskClass::Input => "input task",
        TaskClass::Compositor | TaskClass::GameScope => "display/compositor task",
        TaskClass::Media => "media playback task",
        TaskClass::Recorder => "recording task",
        TaskClass::KernelThread | TaskClass::IrqThread => "kernel/irq helper task",
        TaskClass::Service | TaskClass::NetworkDaemon | TaskClass::StorageDaemon => {
            "system service task"
        }
        _ => "protected task",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_observation_blocks_action() {
        let observation = AutotuneObservation::default();

        assert!(!observation.target_present);
        assert_eq!(observation.primary_situation, SituationKind::Unknown);
        assert!(observation.data_quality.blocks_action());
        assert_eq!(observation.focus_kind, None);
        assert_eq!(observation.focus_confidence, 0.0);
        assert!(observation.focus_roots.is_empty());
        assert!(observation.focus_reasons.is_empty());
        assert!(observation.focus_is_idle_or_unknown());
    }

    fn focus_group_for_observation_test(
        kind: crate::focus::FocusGroupKind,
        confidence: f32,
        roots: Vec<u32>,
        reasons: Vec<String>,
    ) -> crate::focus::ResolvedFocus {
        crate::focus::ResolvedFocus {
            group: crate::focus::FocusGroup {
                kind,
                root_pids: roots.clone(),
                member_pids: roots.clone(),
                primary_pid: roots.first().copied(),
                display_name: format!("{kind:?}"),
                score: 0.75,
                score_breakdown: crate::focus::FocusScoreBreakdown::default(),
                confidence,
                priority_band: crate::focus::PriorityBand::Interactive,
                reasons,
            },
            selected_at_ms: 1000,
            last_confirmed_ms: 1000,
            situation: match kind {
                crate::focus::FocusGroupKind::Game => SituationKind::GameFocused,
                crate::focus::FocusGroupKind::Browser => SituationKind::BrowserFocused,
                crate::focus::FocusGroupKind::Compile => SituationKind::CompileLoad,
                crate::focus::FocusGroupKind::Media => SituationKind::MediaPlayback,
                crate::focus::FocusGroupKind::Recording => SituationKind::Recording,
                crate::focus::FocusGroupKind::VirtualMachine => SituationKind::VirtualMachineLoad,
                crate::focus::FocusGroupKind::Idle => SituationKind::Idle,
                crate::focus::FocusGroupKind::Desktop | crate::focus::FocusGroupKind::Unknown => {
                    SituationKind::Unknown
                }
            },
        }
    }

    #[test]
    fn apply_focus_context_populates_focus_fields_and_primary_situation() {
        let focus = focus_group_for_observation_test(
            crate::focus::FocusGroupKind::Compile,
            0.82,
            vec![1234, 5678],
            vec!["compile group selected".to_owned()],
        );
        let mut observation = AutotuneObservation::default();

        observation.apply_focus_context(Some(&focus));

        assert_eq!(
            observation.focus_kind,
            Some(crate::focus::FocusGroupKind::Compile)
        );
        assert_eq!(observation.focus_confidence, 0.82);
        assert_eq!(observation.focus_roots, vec![1234, 5678]);
        assert_eq!(observation.focus_reasons, vec!["compile group selected"]);
        assert_eq!(observation.primary_situation, SituationKind::CompileLoad);
        assert!(!observation.focus_is_idle_or_unknown());
    }

    #[test]
    fn apply_focus_context_none_clears_focus_fields() {
        let focus = focus_group_for_observation_test(
            crate::focus::FocusGroupKind::Browser,
            0.70,
            vec![2222],
            vec!["browser group selected".to_owned()],
        );
        let mut observation = AutotuneObservation::default();
        observation.apply_focus_context(Some(&focus));

        observation.apply_focus_context(None);

        assert_eq!(observation.focus_kind, None);
        assert_eq!(observation.focus_confidence, 0.0);
        assert!(observation.focus_roots.is_empty());
        assert!(observation.focus_reasons.is_empty());
        assert_eq!(observation.primary_situation, SituationKind::Unknown);
        assert!(observation.focus_is_idle_or_unknown());
    }

    #[test]
    fn focus_reasons_detect_critical_realtime_warning() {
        let observation = AutotuneObservation {
            focus_reasons: vec![
                "safety: critical realtime/input process present pid=55 comm='pipewire'; never lower or deprioritize this task".to_owned(),
            ],
            ..Default::default()
        };

        assert!(observation.focus_has_critical_realtime_warning());
    }

    #[test]
    fn high_data_quality_does_not_block_action() {
        let quality = OnlineDataQuality::High;

        assert!(quality.is_high());
        assert!(!quality.blocks_action());
    }
}
