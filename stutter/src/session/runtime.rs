pub struct MonitorRuntime {
    pub probes: crate::session::probes::ProbeRuntime,
    pub outputs: crate::session::outputs::OutputRuntime,
    pub ui: crate::session::ui::TuiRuntime,
    pub targeting: crate::session::targeting::TargetController,
    pub bus: crate::session::event_bus::MonitorEventBus,
    pub telemetry: crate::session::live_telemetry::LiveTelemetry,
}

impl MonitorRuntime {
    pub fn new(
        probes: crate::session::probes::ProbeRuntime,
        outputs: crate::session::outputs::OutputRuntime,
        ui: crate::session::ui::TuiRuntime,
        targeting: crate::session::targeting::TargetController,
        bus: crate::session::event_bus::MonitorEventBus,
    ) -> Self {
        Self::from_config_parts(probes, outputs, ui, targeting, bus)
    }

    pub fn from_config_parts(
        probes: crate::session::probes::ProbeRuntime,
        outputs: crate::session::outputs::OutputRuntime,
        ui: crate::session::ui::TuiRuntime,
        targeting: crate::session::targeting::TargetController,
        bus: crate::session::event_bus::MonitorEventBus,
    ) -> Self {
        Self {
            probes,
            outputs,
            ui,
            targeting,
            bus,
            telemetry: crate::session::live_telemetry::LiveTelemetry::default(),
        }
    }
}
