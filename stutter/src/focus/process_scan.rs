use super::{
    classify::ProcessIdentity,
    groups::FocusGroupKind,
    snapshot::{FocusProcess, FocusSnapshot},
};
use crate::process_tree::TaskClass as SystemTaskClass;

pub(super) fn contains_game_runtime_text(process: &FocusProcess) -> bool {
    let text = process_identity_text(process);
    text.contains("steamapps")
        || text.contains("pressure-vessel")
        || text.contains("proton")
        || text.contains("wineserver")
}

pub(super) fn is_game_runtime_process(process: &FocusProcess) -> bool {
    let text = process_identity_text(process);
    text.contains("pressure-vessel")
        || text.contains("steam-runtime")
        || text.contains("proton")
        || text.contains("steamapps")
}

pub(super) fn is_game_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::Game
            | SystemTaskClass::GameRenderThread
            | SystemTaskClass::GameWorkerThread
            | SystemTaskClass::WineServer
            | SystemTaskClass::GameScope
    )
}

pub(super) fn is_browser_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserBackground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
    )
}

pub(super) fn is_compile_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::BuildJob
            | SystemTaskClass::Compiler
            | SystemTaskClass::Linker
            | SystemTaskClass::Indexer
            | SystemTaskClass::PackageManager
    )
}

pub(super) fn is_non_service_interactive_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::AudioRealtime
            | SystemTaskClass::Input
            | SystemTaskClass::Game
            | SystemTaskClass::GameRenderThread
            | SystemTaskClass::GameWorkerThread
            | SystemTaskClass::WineServer
            | SystemTaskClass::GameScope
            | SystemTaskClass::Compositor
            | SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
            | SystemTaskClass::Editor
            | SystemTaskClass::Terminal
            | SystemTaskClass::Shell
            | SystemTaskClass::Media
            | SystemTaskClass::Recorder
            | SystemTaskClass::VirtualMachine
    )
}

pub(super) fn is_active_foreground_candidate(process: &FocusProcess) -> bool {
    process.cpu_time_ticks_delta > 0
        || process.read_bytes_delta > 0
        || process.write_bytes_delta > 0
        || process.voluntary_ctxt_switches_delta > 0
        || process.nonvoluntary_ctxt_switches_delta > 0
}

pub(super) fn process_identity_text(process: &FocusProcess) -> String {
    let cgroup_path = process
        .cgroup_path
        .as_ref()
        .map(|path| path.to_string_lossy())
        .unwrap_or_default();

    format!(
        "{} {} {}",
        process.comm.to_ascii_lowercase(),
        process.cmdline.to_ascii_lowercase(),
        cgroup_path.to_ascii_lowercase()
    )
}

pub(super) fn display_name_for_group(
    kind: FocusGroupKind,
    primary: Option<&FocusProcess>,
) -> String {
    if let Some(primary) = primary
        && !primary.comm.is_empty()
    {
        return primary.comm.clone();
    }

    match kind {
        FocusGroupKind::Game => "Game".to_owned(),
        FocusGroupKind::Browser => "Browser".to_owned(),
        FocusGroupKind::Compile => "Compile".to_owned(),
        FocusGroupKind::Media => "Media".to_owned(),
        FocusGroupKind::Recording => "Recording".to_owned(),
        FocusGroupKind::VirtualMachine => "VirtualMachine".to_owned(),
        FocusGroupKind::Desktop => "Desktop".to_owned(),
        FocusGroupKind::Idle => "Idle".to_owned(),
        FocusGroupKind::Unknown => "Unknown".to_owned(),
    }
}
