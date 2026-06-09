use std::sync::Arc;

pub struct EbpfHandles {
    pub loaded: crate::ebpf_loader::LoadedEbpf,
}

pub struct RecorderHandle {
    pub recorder: crate::recorder::LiveRecorder,
}

pub struct ExporterHandles {
    pub prometheus_state: Option<Arc<crate::prometheus::PrometheusState>>,
    pub prometheus_task: Option<tokio::task::JoinHandle<()>>,
    pub otel_exporter: Option<crate::otel::OtelExporterHandle>,
}

pub struct TargetRefreshHandle {
    pub focus_resolver: Option<crate::focus::FocusResolver>,
    pub foreground_resolver: Option<crate::foreground::ForegroundResolver>,
}

pub struct MonitorRuntimeHandles {
    pub ebpf: EbpfHandles,
    pub recorder: Option<RecorderHandle>,
    pub exporters: ExporterHandles,
    pub target_refresh: TargetRefreshHandle,
}

impl EbpfHandles {
    pub fn recorded_activation_warnings(
        &self,
    ) -> Vec<crate::recorder::RecordedProbeActivationWarning> {
        self.loaded
            .activation_plan
            .warnings
            .iter()
            .map(crate::recorder::RecordedProbeActivationWarning::from)
            .collect()
    }
}

#[macro_export]
macro_rules! recorder {
    ($handles:expr) => {
        // invariant: recorder is populated during run
        $handles.recorder.as_ref().unwrap().recorder
    };
}

#[macro_export]
macro_rules! recorder_mut {
    ($handles:expr) => {
        // invariant: recorder is populated during run
        $handles.recorder.as_mut().unwrap().recorder
    };
}
