use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum SystemTaskClass {
    Game,
    GameRenderThread,
    GameWorkerThread,
    WineServer,
    GameScope,
    Compositor,

    AudioRealtime,
    Input,

    BrowserForeground,
    BrowserBackground,
    BrowserRenderer,
    BrowserGpu,
    BrowserNetwork,

    BuildJob,
    Compiler,
    Linker,
    Indexer,
    PackageManager,

    StorageDaemon,
    NetworkDaemon,
    KernelThread,
    IrqThread,

    Editor,
    Terminal,
    Shell,
    Media,
    Recorder,
    VirtualMachine,

    Service,

    #[default]
    Unknown,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PriorityBand {
    CriticalRealtime,
    ForegroundLatency,
    Interactive,
    Throughput,
    Background,
    #[default]
    Unknown,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    pub class: SystemTaskClass,
    pub priority_band: PriorityBand,
    pub confidence: f32,
    pub reasons: Vec<String>,
}
