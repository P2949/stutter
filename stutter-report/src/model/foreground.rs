use serde::{Deserialize, Serialize};
use stutter_core::ids::Pid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ForegroundReportSummary {
    pub enabled: bool,
    pub source: Option<String>,
    pub final_pid: Option<Pid>,
    pub final_app_id: Option<String>,
    pub final_class: Option<String>,
    pub final_title: Option<String>,
    pub final_window_id: Option<String>,
    pub final_workspace: Option<String>,
    pub event_count: u64,
    pub confidence: Option<f32>,
    pub provider_status: Option<String>,
    pub stale_ms: Option<u64>,
    pub reasons: Vec<String>,
}

impl ForegroundReportSummary {
    pub fn is_visible(&self) -> bool {
        self.enabled
            || self.source.is_some()
            || self.final_pid.is_some()
            || self.final_app_id.is_some()
            || self.final_class.is_some()
            || self.final_title.is_some()
            || self.final_window_id.is_some()
            || self.final_workspace.is_some()
            || self.event_count > 0
            || self.confidence.is_some()
            || self.provider_status.is_some()
            || self.stale_ms.is_some()
            || !self.reasons.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FocusReportSummary {
    pub mode: Option<String>,
    pub final_focus: Option<String>,
    pub display_name: Option<String>,
    pub situation: Option<String>,
    pub confidence: Option<f32>,
    pub score: Option<f32>,
    pub roots: Vec<Pid>,
    pub member_pids: Vec<Pid>,
    pub focus_switches: u64,
    pub reasons: Vec<String>,
}

impl FocusReportSummary {
    pub fn is_visible(&self) -> bool {
        self.mode.is_some()
            || self.final_focus.is_some()
            || self.situation.is_some()
            || self.confidence.is_some()
            || !self.roots.is_empty()
            || self.focus_switches > 0
            || !self.reasons.is_empty()
    }
}
