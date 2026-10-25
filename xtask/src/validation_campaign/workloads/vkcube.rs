//! `vkcube` wrapped with MangoHud: the second deterministic frame-producing workload of the
//! StutterScore matrix. `vkmark` and `vkcube` are both Vulkan, so MangoHud's layer logs every
//! frame of both; a GL benchmark was tried first and rejected because MangoHud wrote no frame log
//! for it in this environment.
use std::path::Path;

use super::{CommandSpec, ToolHost, vkmark::mangohud_config};

/// One frozen `vkcube` configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VkcubeConfig {
    pub width: u32,
    pub height: u32,
    /// `VkPresentModeKHR`: 0 immediate, 2 FIFO (vsync).
    pub present_mode: u32,
}

impl VkcubeConfig {
    /// Small and vsync-paced: a light, latency-sensitive frame source.
    pub fn light() -> Self {
        Self {
            width: 640,
            height: 480,
            present_mode: 2,
        }
    }

    /// 4K without vsync: thousands of frames per second, bound by CPU submission.
    pub fn unpaced() -> Self {
        Self {
            width: 3840,
            height: 2160,
            present_mode: 0,
        }
    }

    pub fn identity(&self) -> String {
        format!(
            "vkcube@{}x{}/{}",
            self.width, self.height, self.present_mode
        )
    }
}

/// `mangohud vkcube --c <frames> ...`, logging into `mangohud_dir`. The frame count is a safety
/// bound far above any recording window: the harness stops the process, vkcube has no timer.
pub fn command(config: &VkcubeConfig, seconds: u64, mangohud_dir: &Path) -> CommandSpec {
    CommandSpec::new(
        "mangohud",
        &[
            "vkcube",
            "--c",
            "2000000000",
            "--width",
            &config.width.to_string(),
            "--height",
            &config.height.to_string(),
            "--present_mode",
            &config.present_mode.to_string(),
        ],
    )
    .with_env("MANGOHUD_CONFIG", &mangohud_config(mangohud_dir, seconds))
}

/// `vkcube` and a display must exist; `Err` names exactly what is missing.
pub fn preflight(host: &dyn ToolHost) -> Result<(), String> {
    super::preflight_tools(host, &["vkcube", "mangohud"]).map_err(|p| p.join("; "))?;
    super::vkmark::display_available(host).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_deterministic_and_logs_every_frame() {
        let command = command(&VkcubeConfig::unpaced(), 30, Path::new("/logs"));
        assert_eq!(command.program, "mangohud");
        assert_eq!(&command.args[..2], ["vkcube", "--c"]);
        assert!(
            command
                .args
                .windows(2)
                .any(|w| w == ["--present_mode", "0"])
        );
        assert!(command.args.windows(2).any(|w| w == ["--width", "3840"]));
        assert!(
            command
                .env
                .iter()
                .any(|(k, v)| k == "MANGOHUD_CONFIG" && v.contains("log_interval=0"))
        );
        assert_ne!(
            VkcubeConfig::light().identity(),
            VkcubeConfig::unpaced().identity()
        );
    }
}
