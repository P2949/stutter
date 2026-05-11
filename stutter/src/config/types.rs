use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

pub const TARGET_PIDS_MAX: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum FocusSource {
    Heuristic,
    Foreground,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ForegroundSource {
    Auto,
    Sway,
    Hyprland,
    X11,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CsvStreamTarget {
    File(PathBuf),
    Stdout,
}
