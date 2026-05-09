#![allow(dead_code)]

#[derive(Clone)]
pub(crate) struct TuiRenderSnapshot {
    pub(crate) elapsed_ms: u64,
    pub(crate) drop_counters: crate::ebpf_loader::DropCountersSnapshot,
    pub(crate) tui_state: crate::tui::TuiState,
    pub(crate) active_targets: std::collections::BTreeMap<u32, crate::process_tree::TaskInfo>,
    pub(crate) stats_by_task: std::collections::BTreeMap<u32, crate::metrics::TaskStats>,
    pub(crate) interval_records: Vec<crate::recorder::IntervalRecord>,
    pub(crate) recent_diagnoses: std::collections::VecDeque<crate::diagnosis::LiveDiagnosisEntry>,
    pub(crate) current_focus: Option<crate::focus::ResolvedFocus>,
    pub(crate) current_foreground: Option<crate::foreground::ForegroundWindowSnapshot>,
    pub(crate) focus_switch_count: u64,
    pub(crate) foreground_include_title: bool,
}

pub struct TuiRuntime {
    pub tui_state: crate::tui::TuiState,
    pub terminal: Option<ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>>,
}

impl TuiRuntime {
    pub fn disabled() -> Self {
        Self {
            tui_state: crate::tui::TuiState::default(),
            terminal: None,
        }
    }
}
