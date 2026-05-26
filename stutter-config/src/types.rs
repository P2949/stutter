use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const TARGET_PIDS_MAX: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum FocusSource {
    Heuristic,
    Foreground,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum ForegroundSource {
    Auto,
    Sway,
    Hyprland,
    X11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "kebab-case")]
pub enum WaylandPresentationSource {
    #[default]
    ExternalLog,
    Gamescope,
    SelfTest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CsvStreamTarget {
    File(PathBuf),
    Stdout,
}

#[cfg(test)]
mod tests {
    use super::{FocusSource, ForegroundSource, TARGET_PIDS_MAX, WaylandPresentationSource};

    #[test]
    fn config_type_values_are_stable() {
        assert_eq!(TARGET_PIDS_MAX, 1024);
        assert_eq!(FocusSource::Heuristic, FocusSource::Heuristic);
        assert_eq!(ForegroundSource::Auto, ForegroundSource::Auto);
        assert_eq!(
            WaylandPresentationSource::default(),
            WaylandPresentationSource::ExternalLog
        );
    }
}
