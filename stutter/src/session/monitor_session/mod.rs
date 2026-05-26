//! MonitorSession orchestration and run loop.

use super::*;
use crate::session::ticks::foreground::foreground_event_for_final_metadata;

pub struct MonitorSession {
    pub config: Arc<MonitorConfig>,
    pub handles: crate::session::runtime_handles::MonitorRuntimeHandles,
    pub runtime: MonitorRuntime,

    pub cpu_to_pkg: BTreeMap<u32, String>,

    pub hwmon_reader: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    pub(crate) gpu_engine_reader:
        Option<Arc<std::sync::Mutex<crate::gpu_engine::MultiGpuHwmonReader>>>,
    pub current_focus: Option<ResolvedFocus>,
    pub focus_switch_count: u64,
    pub current_foreground: Option<crate::foreground::ForegroundWindowSnapshot>,
    pub foreground_switch_count: u64,
    pub wayland_presentation_reader: Option<WaylandPresentationLogReader>,
    pub dmabuf_reader: Option<DmaBufLogReader>,

    pub started: Instant,
    pub had_tree_roots: bool,
    pub interval_label: &'static str,
    community_rules: crate::community_rules::CommunityRulesStatus,
}

mod display;
mod event_loop;
mod exporters;
mod probes;
mod shutdown;
mod startup;
mod targets;
