use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::{
    config::model::MonitorConfig, hwmon::HwmonReader, session::MonitorSession,
    session_events::MonitorEvent,
};

#[derive(Clone)]
pub struct MonitorSubsystemConfig {
    pub monitor_config: MonitorConfig,
    pub shared_hwmon: Option<Arc<Mutex<HwmonReader>>>,
    pub event_tx: Option<mpsc::Sender<MonitorEvent>>,
}

pub struct MonitorSubsystem {
    session: MonitorSession,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorShutdownSummary {
    pub event_bus_flush_ran: bool,
    pub otel_exporter_existed: bool,
    pub prometheus_task_existed: bool,
    pub recording_finalization_owned_by_legacy_session: bool,
}

impl MonitorSubsystem {
    pub async fn start(config: MonitorSubsystemConfig) -> anyhow::Result<Self> {
        let session =
            MonitorSession::new(config.monitor_config, config.shared_hwmon, config.event_tx)
                .await?;

        Ok(Self { session })
    }

    pub async fn flush(&mut self) -> anyhow::Result<()> {
        self.session.runtime.bus.flush().await;
        Ok(())
    }

    pub async fn shutdown(mut self) -> anyhow::Result<MonitorShutdownSummary> {
        let otel_exporter_existed = self.session.runtime.outputs.otel_exporter.is_some();
        let prometheus_task_existed = self.session.runtime.outputs.prometheus_task.is_some();

        self.flush().await?;

        Ok(shutdown_summary_from_resource_state(
            true,
            otel_exporter_existed,
            prometheus_task_existed,
        ))
    }

    pub fn session_mut(&mut self) -> &mut MonitorSession {
        &mut self.session
    }
}

fn shutdown_summary_from_resource_state(
    event_bus_flush_ran: bool,
    otel_exporter_existed: bool,
    prometheus_task_existed: bool,
) -> MonitorShutdownSummary {
    MonitorShutdownSummary {
        event_bus_flush_ran,
        otel_exporter_existed,
        prometheus_task_existed,
        recording_finalization_owned_by_legacy_session: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_summary_records_flush_and_existing_resources() {
        let summary = shutdown_summary_from_resource_state(true, true, true);

        assert!(summary.event_bus_flush_ran);
        assert!(summary.otel_exporter_existed);
        assert!(summary.prometheus_task_existed);
        assert!(summary.recording_finalization_owned_by_legacy_session);
    }

    #[test]
    fn shutdown_summary_records_absent_resources_without_taking_recording_finalization() {
        let summary = shutdown_summary_from_resource_state(true, false, false);

        assert!(summary.event_bus_flush_ran);
        assert!(!summary.otel_exporter_existed);
        assert!(!summary.prometheus_task_existed);
        assert!(summary.recording_finalization_owned_by_legacy_session);
    }

    #[test]
    fn monitor_subsystem_config_can_wrap_existing_monitor_session_inputs() {
        let (event_tx, _event_rx) = mpsc::channel(1);
        let config = MonitorSubsystemConfig {
            monitor_config: MonitorConfig::default(),
            shared_hwmon: None,
            event_tx: Some(event_tx),
        };

        assert_eq!(config.monitor_config, MonitorConfig::default());
        assert!(config.shared_hwmon.is_none());
        assert!(config.event_tx.is_some());
    }

    #[test]
    fn shutdown_summary_shape_is_stable_for_daemon_runtime_consumers() {
        let summary = MonitorShutdownSummary {
            event_bus_flush_ran: true,
            otel_exporter_existed: false,
            prometheus_task_existed: true,
            recording_finalization_owned_by_legacy_session: true,
        };

        assert_eq!(
            summary,
            MonitorShutdownSummary {
                event_bus_flush_ran: true,
                otel_exporter_existed: false,
                prometheus_task_existed: true,
                recording_finalization_owned_by_legacy_session: true,
            }
        );
    }
}
