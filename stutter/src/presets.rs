use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Gaming,
    Recording,
    Diagnosis,
    PrimeDisplayPath,
    Lightweight,
}

pub const VALID_PRESETS: &[&str] = &[
    "gaming",
    "recording",
    "diagnosis",
    "prime-display-path",
    "lightweight",
];

impl FromStr for Preset {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "gaming" => Ok(Self::Gaming),
            "recording" => Ok(Self::Recording),
            "diagnosis" => Ok(Self::Diagnosis),
            "prime-display-path" => Ok(Self::PrimeDisplayPath),
            "lightweight" => Ok(Self::Lightweight),
            other => anyhow::bail!(
                "unknown preset {:?}; valid presets: {}",
                other,
                VALID_PRESETS.join(", ")
            ),
        }
    }
}

impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Preset::Gaming => "gaming",
            Preset::Recording => "recording",
            Preset::Diagnosis => "diagnosis",
            Preset::PrimeDisplayPath => "prime-display-path",
            Preset::Lightweight => "lightweight",
        };

        f.write_str(name)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetDefaults {
    pub hwmon: Option<bool>,
    pub cpu_freq: Option<bool>,
    pub faults: Option<bool>,
    pub stat_wait: Option<bool>,
    pub block_io: Option<bool>,
    pub runtime_slices: Option<bool>,
    pub irq_latency: Option<bool>,
    pub kms_timing: Option<bool>,
    pub drm_fence_latency: Option<bool>,
    pub wayland_presentation: Option<bool>,
    pub foreground_window: Option<bool>,
    pub gpu_engine_sampling: Option<bool>,
    pub display_topology: Option<bool>,
}

impl PresetDefaults {
    pub fn into_monitor_config_layer(self) -> crate::config::layer::MonitorConfigLayer {
        crate::config::layer::MonitorConfigLayer::from_preset_defaults(self)
    }
}

impl Preset {
    pub fn defaults(self) -> PresetDefaults {
        match self {
            Preset::Gaming => PresetDefaults {
                hwmon: Some(true),
                cpu_freq: Some(true),
                faults: Some(true),
                stat_wait: Some(true),
                block_io: None,
                runtime_slices: Some(false),
                irq_latency: None,
                ..PresetDefaults::default()
            },
            Preset::Recording => PresetDefaults {
                hwmon: Some(true),
                cpu_freq: Some(true),
                faults: Some(true),
                stat_wait: Some(true),
                block_io: Some(true),
                runtime_slices: Some(false),
                irq_latency: None,
                ..PresetDefaults::default()
            },
            Preset::Diagnosis => PresetDefaults {
                hwmon: Some(true),
                cpu_freq: Some(true),
                faults: Some(true),
                stat_wait: Some(true),
                block_io: Some(true),
                runtime_slices: Some(true),
                irq_latency: None,
                kms_timing: None,
                drm_fence_latency: None,
                wayland_presentation: None,
                foreground_window: None,
                gpu_engine_sampling: None,
                display_topology: None,
            },
            Preset::PrimeDisplayPath => PresetDefaults {
                hwmon: Some(true),
                cpu_freq: Some(true),
                faults: Some(true),
                stat_wait: Some(true),
                block_io: Some(true),
                runtime_slices: Some(true),
                irq_latency: None,
                kms_timing: Some(true),
                drm_fence_latency: Some(true),
                wayland_presentation: None,
                foreground_window: Some(true),
                gpu_engine_sampling: Some(true),
                display_topology: Some(true),
            },
            Preset::Lightweight => PresetDefaults {
                hwmon: Some(false),
                cpu_freq: Some(false),
                faults: Some(false),
                stat_wait: Some(false),
                block_io: Some(false),
                runtime_slices: Some(false),
                irq_latency: Some(false),
                kms_timing: Some(false),
                drm_fence_latency: Some(false),
                wayland_presentation: Some(false),
                foreground_window: Some(false),
                gpu_engine_sampling: Some(false),
                display_topology: Some(false),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_preset_errors_with_valid_names() {
        let err = "diag".parse::<Preset>().unwrap_err();
        let msg = err.to_string();

        assert!(msg.contains("unknown preset"));
        assert!(msg.contains("gaming"));
        assert!(msg.contains("recording"));
        assert!(msg.contains("diagnosis"));
        assert!(msg.contains("prime-display-path"));
        assert!(msg.contains("lightweight"));
    }

    #[test]
    fn prime_display_path_preset_enables_display_path_evidence() {
        let defaults = Preset::PrimeDisplayPath.defaults();

        assert_eq!(defaults.hwmon, Some(true));
        assert_eq!(defaults.runtime_slices, Some(true));
        assert_eq!(defaults.kms_timing, Some(true));
        assert_eq!(defaults.drm_fence_latency, Some(true));
        assert_eq!(defaults.wayland_presentation, None);
        assert_eq!(defaults.foreground_window, Some(true));
        assert_eq!(defaults.gpu_engine_sampling, Some(true));
        assert_eq!(defaults.display_topology, Some(true));
        assert_eq!(Preset::PrimeDisplayPath.to_string(), "prime-display-path");

        let layer = defaults.into_monitor_config_layer();
        assert_eq!(layer.kms_timing, Some(true));
        assert_eq!(layer.drm_fence_latency, Some(true));
        assert_eq!(layer.foreground_window, Some(true));
        assert_eq!(layer.gpu_engine_sampling, Some(true));
        assert_eq!(layer.display_topology, Some(true));
    }
}
