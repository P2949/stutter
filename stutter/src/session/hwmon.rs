//! Hardware monitor reader setup for monitor sessions.

use std::sync::Arc;

use log::warn;

use crate::{config::model::MonitorConfig, hwmon};

pub(crate) struct HwmonRuntime {
    pub(crate) reader: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
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

        if config.probes.hwmon && reader.is_none() {
            warn!("hwmon_requested_but_no_gpu_hwmon_found");
        }

        Self { reader }
    }
}
