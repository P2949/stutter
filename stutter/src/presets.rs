use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Gaming,
    Recording,
    Diagnosis,
    Lightweight,
}

pub const VALID_PRESETS: &[&str] = &["gaming", "recording", "diagnosis", "lightweight"];

impl FromStr for Preset {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "gaming" => Ok(Self::Gaming),
            "recording" => Ok(Self::Recording),
            "diagnosis" => Ok(Self::Diagnosis),
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
            },
            Preset::Recording => PresetDefaults {
                hwmon: Some(true),
                cpu_freq: Some(true),
                faults: Some(true),
                stat_wait: Some(true),
                block_io: Some(true),
                runtime_slices: Some(false),
                irq_latency: None,
            },
            Preset::Diagnosis => PresetDefaults {
                hwmon: Some(true),
                cpu_freq: Some(true),
                faults: Some(true),
                stat_wait: Some(true),
                block_io: Some(true),
                runtime_slices: Some(true),
                irq_latency: None,
            },
            Preset::Lightweight => PresetDefaults {
                hwmon: Some(false),
                cpu_freq: Some(false),
                faults: Some(false),
                stat_wait: Some(false),
                block_io: Some(false),
                runtime_slices: Some(false),
                irq_latency: Some(false),
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
        assert!(msg.contains("lightweight"));
    }
}
