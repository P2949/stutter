//! `vkmark` wrapped with MangoHud: the deterministic frame-producing workload. No mouse or
//! keyboard automation is involved; the benchmark runs unattended for a fixed duration and
//! MangoHud logs per-frame times that Stutter ingests with `--mangohud-log`.
use std::path::{Path, PathBuf};

use super::{CommandSpec, ToolHost};
use crate::validation_campaign::scenarios::model::VkmarkParams;

/// One frozen vkmark configuration; every capture of a series uses exactly the same one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VkmarkConfig {
    pub scene: String,
    pub size: String,
    pub present_mode: String,
}

impl VkmarkConfig {
    pub fn light(params: &VkmarkParams) -> Self {
        Self {
            scene: params.light_scene.clone(),
            size: params.light_size.clone(),
            present_mode: params.present_mode.clone(),
        }
    }

    pub fn heavy(params: &VkmarkParams) -> Self {
        Self {
            scene: params.heavy_scene.clone(),
            size: params.heavy_size.clone(),
            present_mode: params.present_mode.clone(),
        }
    }

    /// Stable identity string (scene, size, present mode) for fingerprints and provenance.
    pub fn identity(&self) -> String {
        format!("{}@{}/{}", self.scene, self.size, self.present_mode)
    }
}

/// MangoHud settings that log every frame to `output_dir` for `seconds`.
pub fn mangohud_config(output_dir: &Path, seconds: u64) -> String {
    format!(
        "autostart_log=1,log_duration={seconds},log_interval=0,output_folder={}",
        output_dir.display()
    )
}

/// `mangohud vkmark -b <scene>:duration=<s> -s <size> -p <mode>`, logging into `mangohud_dir`.
pub fn command(config: &VkmarkConfig, seconds: u64, mangohud_dir: &Path) -> CommandSpec {
    CommandSpec::new(
        "mangohud",
        &[
            "vkmark",
            "-b",
            &format!("{}:duration={seconds}.0", config.scene),
            "-s",
            &config.size,
            "-p",
            &config.present_mode,
        ],
    )
    .with_env("MANGOHUD_CONFIG", &mangohud_config(mangohud_dir, seconds))
}

/// Scene names from `vkmark --list-scenes`, whose scenes are the `[Scene] <name>` lines (the
/// `[Option]` and `Description` lines beneath them are not scenes).
pub fn parse_scene_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("[Scene]"))
        .map(|name| name.trim().to_owned())
        .filter(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .collect()
}

/// The newest MangoHud CSV in `dir`: the log of the run that just started.
pub fn newest_log(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "csv"))
        .filter_map(|path| Some((path.metadata().ok()?.modified().ok()?, path)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// A usable window system is required; without one the frame series is `BlockedResource`.
pub fn display_available(host: &dyn ToolHost) -> Result<String, String> {
    if host.env_var("WAYLAND_DISPLAY").is_some() {
        Ok("wayland".to_owned())
    } else if host.env_var("DISPLAY").is_some() {
        Ok("x11".to_owned())
    } else {
        Err("no display session (neither WAYLAND_DISPLAY nor DISPLAY is set)".to_owned())
    }
}

/// What preflight learned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VkmarkFacts {
    pub window_system: String,
    pub scenes: Vec<String>,
}

/// Verifies the tools, the window system and that the frozen scene is available.
pub fn preflight(host: &dyn ToolHost, wanted: &[&str]) -> Result<VkmarkFacts, String> {
    for tool in ["vkmark", "mangohud"] {
        host.find(tool)
            .ok_or_else(|| format!("{tool} is not on PATH"))?;
    }
    let window_system = display_available(host)?;
    let scenes = parse_scene_list(&host.output(&CommandSpec::new("vkmark", &["--list-scenes"]))?);
    for scene in wanted {
        if !scenes.iter().any(|s| s == scene) {
            return Err(format!(
                "vkmark has no scene {scene}; available: {}",
                scenes.join(", ")
            ));
        }
    }
    Ok(VkmarkFacts {
        window_system,
        scenes,
    })
}
