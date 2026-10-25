//! The StutterScore matrix: profiles, levels, repetitions and the order they are run in.
use anyhow::ensure;
use serde::Deserialize;

use crate::validation_campaign::scenarios::model::{Target, VkmarkParams};

pub const CONFIG_FILE: &str = "docs/validation/scenarios/score-matrix.toml";
pub const STUDY_ROOT: &str = ".llm-runs/studies/score-matrix";

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MatrixConfig {
    pub schema_version: u32,
    pub seconds: u64,
    pub repetitions: usize,
    pub levels_ms: Vec<u32>,
    pub period_ms: u64,
    pub failed_attempts_per_capture: usize,
    pub vkmark: VkmarkParams,
}

impl MatrixConfig {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let config: Self = toml::from_str(text)?;
        ensure!(config.schema_version == 1, "unsupported matrix schema");
        ensure!(
            config.levels_ms.first() == Some(&0)
                && config.levels_ms.len() >= 4
                && config.levels_ms.len().is_multiple_of(2)
                && config.levels_ms.windows(2).all(|w| w[0] < w[1]),
            "levels must start at the 0 ms control, ascend, and be an even count of at least four"
        );
        ensure!(
            (5..=12).contains(&config.repetitions)
                && (20..=120).contains(&config.seconds)
                && config.period_ms > u64::from(*config.levels_ms.last().unwrap_or(&0))
                && config.failed_attempts_per_capture > 0,
            "repetitions need 5..=12, seconds 20..=120 and a period above the longest burst"
        );
        Ok(config)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Profile {
    VkmarkLight,
    VkmarkHeavy,
    VkcubeLight,
    VkcubeHeavy,
}

impl Profile {
    pub const ALL: [Self; 4] = [
        Self::VkmarkLight,
        Self::VkmarkHeavy,
        Self::VkcubeLight,
        Self::VkcubeHeavy,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::VkmarkLight => "vkmark-light",
            Self::VkmarkHeavy => "vkmark-heavy",
            Self::VkcubeLight => "vkcube-light",
            Self::VkcubeHeavy => "vkcube-heavy",
        }
    }

    pub fn target(self) -> Target {
        match self {
            Self::VkmarkLight => Target::VkmarkLight,
            Self::VkmarkHeavy => Target::VkmarkGpuHeavy,
            Self::VkcubeLight => Target::VkcubeLight,
            Self::VkcubeHeavy => Target::VkcubeUnpaced,
        }
    }

    /// Coalesced wakeup records tolerated per capture: a GPU-saturating or unpaced target makes
    /// driver threads wake in bursts. Declared up front and recorded.
    pub fn wakeup_allowance(self) -> u64 {
        match self {
            Self::VkmarkLight | Self::VkcubeLight => 16,
            Self::VkmarkHeavy | Self::VkcubeHeavy => 1024,
        }
    }
}

/// The order of the `n` levels in repetition `k`: a Williams balanced Latin square (every level
/// follows every other equally often over `n` repetitions), rotated by `k`.
pub fn level_order(n: usize, repetition: usize) -> Vec<usize> {
    let mut base = vec![0];
    let (mut low, mut high) = (1, n - 1);
    while base.len() < n {
        base.push(low);
        low += 1;
        if base.len() < n {
            base.push(high);
            high -= 1;
        }
    }
    base.into_iter().map(|b| (b + repetition) % n).collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Slot {
    pub profile: Profile,
    pub repetition: usize,
    pub position: usize,
    pub level_ms: u32,
}

/// Every capture in run order: repetitions outermost and profiles interleaved, so slow drift of
/// the host spreads over all profiles instead of landing on one.
pub fn slots(config: &MatrixConfig) -> Vec<Slot> {
    let n = config.levels_ms.len();
    let mut out = Vec::new();
    for repetition in 0..config.repetitions {
        for profile in Profile::ALL {
            for (position, index) in level_order(n, repetition).into_iter().enumerate() {
                out.push(Slot {
                    profile,
                    repetition,
                    position,
                    level_ms: config.levels_ms[index],
                });
            }
        }
    }
    out
}
