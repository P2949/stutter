#![allow(dead_code)]

use std::path::PathBuf;

pub mod controller;
pub mod decision;
pub mod observation;
pub mod quality;
pub mod replay;
pub mod state;

#[derive(Debug, Clone)]
pub struct AutotuneCommandInput {
    pub config: Option<PathBuf>,
    pub watch_process: Option<String>,
    pub tree_pid: Option<u32>,
    pub profiles: Option<PathBuf>,
    pub mode: String,
    pub decision_log: Option<PathBuf>,
    pub duration_seconds: Option<u64>,
    pub summary_ms: u64,
    pub preset: String,
    pub hwmon: bool,
    pub mangohud_log: Option<PathBuf>,
}

pub async fn autotune_command(input: AutotuneCommandInput) -> anyhow::Result<()> {
    match input.mode.as_str() {
        "observe" | "suggest" => {
            println!(
                "autotune mode={} parsed; live autotune controller is not implemented yet; no actions applied",
                input.mode
            );
            println!(
                "autotune config={:?} watch_process={:?} tree_pid={:?} profiles={:?} decision_log={:?} duration_seconds={:?} summary_ms={} preset={} hwmon={} mangohud_log={:?}",
                input.config,
                input.watch_process,
                input.tree_pid,
                input.profiles,
                input.decision_log,
                input.duration_seconds,
                input.summary_ms,
                input.preset,
                input.hwmon,
                input.mangohud_log
            );
            Ok(())
        }
        _ => {
            anyhow::bail!("apply mode is not implemented yet; use --mode observe or --mode suggest")
        }
    }
}
