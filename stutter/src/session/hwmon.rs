//! Hardware monitor reader setup for monitor sessions.

use std::sync::Arc;

use log::warn;

use crate::{config::model::MonitorConfig, gpu_engine, hwmon};

pub(crate) struct HwmonRuntime {
    pub(crate) reader: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    pub(crate) engine_reader: Option<Arc<std::sync::Mutex<gpu_engine::MultiGpuHwmonReader>>>,
}

impl HwmonRuntime {
    pub(crate) fn begin(
        config: &MonitorConfig,
        shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    ) -> Self {
        let reader = if !config.probes.hwmon {
            None
        } else if let Some(shared) = shared_hwmon {
            Some(shared)
        } else {
            hwmon::HwmonReader::discover_with_options(
                config.hwmon.root.as_deref(),
                config.hwmon.drm_card.as_deref(),
                config.hwmon.render_node.as_deref(),
            )
            .map(|r| Arc::new(std::sync::Mutex::new(r)))
        };

        let engine_reader = gpu_engine::MultiGpuHwmonReader::discover(config)
            .map(|reader| Arc::new(std::sync::Mutex::new(reader)));

        if config.probes.hwmon && reader.is_none() {
            warn!("hwmon_requested_but_no_gpu_hwmon_found");
        }
        if config.probes.gpu_engine_sampling && engine_reader.is_none() {
            warn!("gpu_engine_sampling_requested_but_no_gpu_hwmon_found");
        }

        Self {
            reader,
            engine_reader,
        }
    }
}
