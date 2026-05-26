use std::collections::VecDeque;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
};

use crate::{
    autotune::tui_panel::load_default_autotune_tui_panel_snapshot,
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    focus::ResolvedFocus,
    foreground::ForegroundWindowSnapshot,
    metrics::{IntervalRecord, TaskStatsMap},
    process_tree::TaskClass,
};

mod autotune_panel;
mod cpu_heat;
mod diagnosis;
mod model;
mod sparkline;
mod status;
mod task_table;
mod terminal;

use autotune_panel::render_autotune_panel;
use cpu_heat::render_cpu_heat;
use diagnosis::render_diagnoses;
use model::TuiModel;
use sparkline::render_sparkline;
use status::render_status_bar;
use task_table::render_task_table;
pub use terminal::{init_terminal, restore_terminal};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortField {
    MaxLatency,
    AvgLatency,
    Samples,
}

impl SortField {
    pub fn next(self) -> Self {
        match self {
            Self::MaxLatency => Self::AvgLatency,
            Self::AvgLatency => Self::Samples,
            Self::Samples => Self::MaxLatency,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::MaxLatency => "Max Latency",
            Self::AvgLatency => "Avg Latency",
            Self::Samples => "Samples",
        }
    }
}

#[derive(Clone)]
pub struct TuiState {
    pub sort_field: SortField,
    pub filter_class: Option<TaskClass>,
    pub paused: bool,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            sort_field: SortField::MaxLatency,
            filter_class: None,
            paused: false,
        }
    }
}

impl TuiState {
    /// Cycle through `None -> Unknown -> Game -> ... -> None`.
    pub fn next_filter_class(&mut self) {
        const CLASSES: &[TaskClass] = &[
            TaskClass::Unknown,
            TaskClass::Game,
            TaskClass::GameRenderThread,
            TaskClass::GameWorkerThread,
            TaskClass::GameHelper,
            TaskClass::Launcher,
            TaskClass::WineServer,
            TaskClass::GameScope,
            TaskClass::Compositor,
            TaskClass::AudioRealtime,
            TaskClass::BrowserForeground,
            TaskClass::SteamRuntime,
            TaskClass::Helper,
            TaskClass::Service,
        ];
        if let Some(current) = self.filter_class {
            let pos = CLASSES.iter().position(|c| *c == current).unwrap_or(0);
            if pos + 1 < CLASSES.len() {
                self.filter_class = Some(CLASSES[pos + 1]);
            } else {
                self.filter_class = None;
            }
        } else {
            self.filter_class = Some(CLASSES[0]);
        }
    }
}

pub struct TuiRenderInput<'a> {
    pub state: &'a TuiState,
    pub active_targets: &'a crate::process_tree::TaskMap,
    pub stats_by_task: &'a TaskStatsMap,
    pub interval_records: &'a [IntervalRecord],
    pub recent_diagnoses: &'a VecDeque<LiveDiagnosisEntry>,
    pub elapsed_ms: u128,
    pub drop_counters: &'a DropCountersSnapshot,
    pub current_focus: Option<&'a ResolvedFocus>,
    pub current_foreground: Option<&'a ForegroundWindowSnapshot>,
    pub focus_switch_count: u64,
    pub foreground_include_title: bool,
}

pub fn render_tui(f: &mut Frame, input: TuiRenderInput<'_>) {
    let autotune_snapshot = load_default_autotune_tui_panel_snapshot();
    let model = TuiModel::from_render_input(&input, Some(&autotune_snapshot));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(10),
            Constraint::Length(8),
            Constraint::Length(8),
        ])
        .split(f.area());

    render_status_bar(f, chunks[0], &model);
    render_task_table(f, &model.task_rows, chunks[1]);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    render_sparkline(f, &model.sparkline_ms, bottom[0]);
    render_cpu_heat(f, &model.cpu_heat, bottom[1]);

    let lower = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[3]);

    render_autotune_panel(f, &model.autotune, lower[0]);
    render_diagnoses(f, &model.diagnoses, lower[1]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_field_cycles() {
        let mut s = SortField::MaxLatency;
        s = s.next();
        assert_eq!(s, SortField::AvgLatency);
        s = s.next();
        assert_eq!(s, SortField::Samples);
        s = s.next();
        assert_eq!(s, SortField::MaxLatency);
    }

    #[test]
    fn filter_class_cycles_through_all_and_wraps() {
        let mut state = TuiState::default();
        assert_eq!(state.filter_class, None);
        state.next_filter_class();
        assert_eq!(state.filter_class, Some(TaskClass::Unknown));

        let mut count = 0;
        while state.filter_class.is_some() && count < 100 {
            state.next_filter_class();
            count += 1;
        }

        assert_eq!(state.filter_class, None);
        assert!(count > 5);
    }
}
