use super::super::{groups::FocusGroupKind, snapshot::FocusProcess};
use crate::process_tree::TaskClass as SystemTaskClass;

pub(crate) fn focus_group_kind_for_class(class: SystemTaskClass) -> FocusGroupKind {
    match class {
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::GameScope => FocusGroupKind::Game,

        SystemTaskClass::BrowserForeground
        | SystemTaskClass::BrowserBackground
        | SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::BrowserNetwork => FocusGroupKind::Browser,

        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::Indexer
        | SystemTaskClass::PackageManager => FocusGroupKind::Compile,

        SystemTaskClass::Media => FocusGroupKind::Media,
        SystemTaskClass::Recorder => FocusGroupKind::Recording,
        SystemTaskClass::VirtualMachine => FocusGroupKind::VirtualMachine,

        SystemTaskClass::Compositor | SystemTaskClass::AudioRealtime | SystemTaskClass::Input => {
            FocusGroupKind::Desktop
        }

        SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Unknown => FocusGroupKind::Unknown,

        SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread
        | SystemTaskClass::Service => FocusGroupKind::Idle,
        _ => FocusGroupKind::Unknown,
    }
}

pub(crate) fn process_focus_score(process: &FocusProcess) -> f32 {
    let class_base = match process.classification.class {
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input => 90.0,
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::GameScope => 80.0,
        SystemTaskClass::Compositor | SystemTaskClass::BrowserForeground => 70.0,
        SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::BrowserNetwork
        | SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Media
        | SystemTaskClass::Recorder
        | SystemTaskClass::VirtualMachine => 50.0,
        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::Indexer
        | SystemTaskClass::PackageManager => 35.0,
        SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread
        | SystemTaskClass::Service
        | SystemTaskClass::BrowserBackground => 15.0,
        SystemTaskClass::Unknown => 0.0,
        _ => 10.0,
    };

    let cpu_score = process.cpu_time_ticks_delta as f32;
    let io_score = (process
        .read_bytes_delta
        .saturating_add(process.write_bytes_delta) as f32)
        / 1_048_576.0;
    let ctxt_score = (process
        .voluntary_ctxt_switches_delta
        .saturating_add(process.nonvoluntary_ctxt_switches_delta) as f32)
        * 0.05;

    class_base + process.classification.confidence + cpu_score + io_score + ctxt_score
}
