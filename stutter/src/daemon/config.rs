use std::{
    collections::BTreeSet,
    fmt,
    path::{Component, Path, PathBuf},
    str::FromStr,
};

use serde::{Deserialize, Serialize};

use crate::{
    actions::SafetyClass,
    daemon::{
        health::SystemHealthThresholds,
        policy::{ActionSource, DaemonMode},
    },
};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonPreset {
    #[default]
    ObserveOnly,
    GamingLowRisk,
    GamingLaptopSafe,
    WorkstationLowRisk,
    DebugAggressive,
}

impl DaemonPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObserveOnly => "observe-only",
            Self::GamingLowRisk => "gaming-low-risk",
            Self::GamingLaptopSafe => "gaming-laptop-safe",
            Self::WorkstationLowRisk => "workstation-low-risk",
            Self::DebugAggressive => "debug-aggressive",
        }
    }
}

impl fmt::Display for DaemonPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DaemonPreset {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "observe-only" => Ok(Self::ObserveOnly),
            "gaming-low-risk" => Ok(Self::GamingLowRisk),
            "gaming-laptop-safe" => Ok(Self::GamingLaptopSafe),
            "workstation-low-risk" => Ok(Self::WorkstationLowRisk),
            "debug-aggressive" => Ok(Self::DebugAggressive),
            other => anyhow::bail!(
                "invalid daemon preset {other:?}; valid values are observe-only, gaming-low-risk, gaming-laptop-safe, workstation-low-risk, debug-aggressive"
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonConfig {
    pub preset: DaemonPreset,
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub target: DaemonTargetConfig,
    pub safety: DaemonSafetyConfig,
    pub health: DaemonHealthConfig,
    pub retention: DaemonRetentionConfig,
    pub remote: DaemonRemoteConfig,
    pub autotune: DaemonAutotuneConfig,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self::from_preset(DaemonPreset::ObserveOnly, ActionSource::Cli)
    }
}

impl DaemonConfig {
    pub fn from_preset(preset: DaemonPreset, source: ActionSource) -> Self {
        let mut config = Self {
            preset,
            mode: DaemonMode::Observe,
            source,
            target: DaemonTargetConfig::default(),
            safety: DaemonSafetyConfig::default(),
            health: DaemonHealthConfig::default(),
            retention: DaemonRetentionConfig::default(),
            remote: DaemonRemoteConfig::default(),
            autotune: DaemonAutotuneConfig::default(),
        };

        config.apply_preset(preset);
        config
    }

    pub fn apply_preset(&mut self, preset: DaemonPreset) {
        self.preset = preset;
        self.safety = DaemonSafetyConfig::default();
        self.health = DaemonHealthConfig::default();
        self.retention = DaemonRetentionConfig::default();
        self.remote = DaemonRemoteConfig::default();
        self.autotune = DaemonAutotuneConfig::default();

        match preset {
            DaemonPreset::ObserveOnly => {
                self.mode = DaemonMode::Observe;
                self.safety.max_safety_class = SafetyClass::ObserveOnly;
                self.safety.allowed_action_classes = safety_classes_up_to(SafetyClass::ObserveOnly);
                self.safety.min_confidence = 0.0;
            }
            DaemonPreset::GamingLowRisk => {
                self.mode = DaemonMode::ApplyLowRisk;
                self.safety.max_safety_class = SafetyClass::ReversibleLowRisk;
                self.safety.allowed_action_classes =
                    safety_classes_up_to(SafetyClass::ReversibleLowRisk);
                self.safety
                    .enabled_action_families
                    .insert("cpu_affinity_profile".to_owned());
                self.safety.min_confidence = 0.85;
                self.autotune.candidate_window_seconds = 30;
                self.autotune.washout_seconds = 10;
                self.retention.max_history_events = 20_000;
            }
            DaemonPreset::GamingLaptopSafe => {
                self.mode = DaemonMode::ApplyLowRisk;
                self.safety.max_safety_class = SafetyClass::ReversibleLowRisk;
                self.safety.allowed_action_classes =
                    safety_classes_up_to(SafetyClass::ReversibleLowRisk);
                self.safety
                    .enabled_action_families
                    .insert("cpu_affinity_profile".to_owned());
                self.safety.min_confidence = 0.92;
                self.health.max_cpu_temp_celsius = 82;
                self.health.max_gpu_temp_celsius = 84;
                self.autotune.candidate_window_seconds = 45;
                self.autotune.washout_seconds = 15;
                self.retention.max_history_events = 12_000;
            }
            DaemonPreset::WorkstationLowRisk => {
                self.mode = DaemonMode::ApplyLowRisk;
                self.safety.max_safety_class = SafetyClass::ReversibleLowRisk;
                self.safety.allowed_action_classes =
                    safety_classes_up_to(SafetyClass::ReversibleLowRisk);
                self.safety
                    .enabled_action_families
                    .insert("cpu_affinity_profile".to_owned());
                self.safety.min_confidence = 0.88;
                self.autotune.candidate_window_seconds = 60;
                self.autotune.washout_seconds = 15;
                self.retention.max_history_events = 30_000;
            }
            DaemonPreset::DebugAggressive => {
                self.mode = DaemonMode::ApplyMediumRisk;
                self.safety.max_safety_class = SafetyClass::ReversibleMediumRisk;
                self.safety.allowed_action_classes =
                    safety_classes_up_to(SafetyClass::ReversibleMediumRisk);
                self.safety.enabled_action_families.extend(
                    ["cpu_affinity_profile", "nice", "ionice", "uclamp"].map(str::to_owned),
                );
                self.safety.min_confidence = 0.75;
                self.autotune.candidate_window_seconds = 20;
                self.autotune.washout_seconds = 5;
                self.retention.max_history_events = 50_000;
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonHealthConfig {
    pub max_cpu_temp_celsius: u32,
    pub max_gpu_temp_celsius: u32,
    pub min_disk_available_bytes: u64,
    pub max_memory_pressure_some_avg10_percent: f32,
}

impl Default for DaemonHealthConfig {
    fn default() -> Self {
        let thresholds = SystemHealthThresholds::default();

        Self {
            max_cpu_temp_celsius: (thresholds.max_cpu_temp_millidegrees / 1000) as u32,
            max_gpu_temp_celsius: (thresholds.max_gpu_temp_millidegrees / 1000) as u32,
            min_disk_available_bytes: thresholds.min_disk_available_bytes,
            max_memory_pressure_some_avg10_percent: thresholds
                .max_memory_pressure_some_avg10_millipercent
                as f32
                / 1000.0,
        }
    }
}

impl DaemonHealthConfig {
    pub fn thresholds(&self) -> SystemHealthThresholds {
        let defaults = SystemHealthThresholds::default();

        SystemHealthThresholds {
            max_cpu_temp_millidegrees: i64::from(self.max_cpu_temp_celsius) * 1000,
            max_gpu_temp_millidegrees: i64::from(self.max_gpu_temp_celsius) * 1000,
            min_disk_available_bytes: self.min_disk_available_bytes,
            max_memory_pressure_some_avg10_millipercent: (self
                .max_memory_pressure_some_avg10_percent
                * 1000.0)
                .round() as u32,
            max_load_per_cpu_milli: defaults.max_load_per_cpu_milli,
            max_ebpf_dropped_events: defaults.max_ebpf_dropped_events,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonTargetConfig {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub watch_process: Option<String>,
    pub require_explicit_target: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonSafetyConfig {
    pub max_safety_class: SafetyClass,
    pub allowed_action_classes: BTreeSet<SafetyClass>,
    pub enabled_action_families: BTreeSet<String>,
    pub denied_action_families: BTreeSet<String>,
    pub cgroup_targets: DaemonCgroupTargetsConfig,
    pub allow_system_wide_suggestions: bool,
    pub allow_system_wide_apply: bool,
    pub allow_high_risk: bool,
    pub allow_persistent_effects: bool,
    pub min_confidence: f32,
}

impl Default for DaemonSafetyConfig {
    fn default() -> Self {
        let mut allowed_action_classes = BTreeSet::new();
        allowed_action_classes.insert(SafetyClass::ObserveOnly);

        Self {
            max_safety_class: SafetyClass::ObserveOnly,
            allowed_action_classes,
            enabled_action_families: BTreeSet::new(),
            denied_action_families: BTreeSet::new(),
            cgroup_targets: DaemonCgroupTargetsConfig::default(),
            allow_system_wide_suggestions: false,
            allow_system_wide_apply: false,
            allow_high_risk: false,
            allow_persistent_effects: false,
            min_confidence: 0.0,
        }
    }
}

fn safety_classes_up_to(max: SafetyClass) -> BTreeSet<SafetyClass> {
    [
        SafetyClass::ObserveOnly,
        SafetyClass::ReversibleLowRisk,
        SafetyClass::ReversibleMediumRisk,
        SafetyClass::HighRisk,
    ]
    .into_iter()
    .filter(|class| class <= &max)
    .collect()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonCgroupTargetsConfig {
    pub interactive_cgroup: Option<PathBuf>,
    pub background_cgroup: Option<PathBuf>,
    pub game_cgroup: Option<PathBuf>,
    pub compile_cgroup: Option<PathBuf>,
}

impl DaemonCgroupTargetsConfig {
    pub fn is_empty(&self) -> bool {
        self.interactive_cgroup.is_none()
            && self.background_cgroup.is_none()
            && self.game_cgroup.is_none()
            && self.compile_cgroup.is_none()
    }

    pub fn target_for_role(&self, role: CgroupTargetRole) -> Option<&Path> {
        match role {
            CgroupTargetRole::Interactive => self.interactive_cgroup.as_deref(),
            CgroupTargetRole::Background => self.background_cgroup.as_deref(),
            CgroupTargetRole::Game => self.game_cgroup.as_deref(),
            CgroupTargetRole::Compile => self.compile_cgroup.as_deref(),
        }
    }

    pub fn contains_path(&self, path: &Path) -> bool {
        let Ok(candidate) = normalize_cgroup_target_path(path) else {
            return false;
        };

        self.named_targets().into_iter().any(|(_, target)| {
            normalize_cgroup_target_path(target).is_ok_and(|known| known == candidate)
        })
    }

    pub fn named_targets(&self) -> Vec<(&'static str, &Path)> {
        [
            (
                CgroupTargetRole::Interactive.as_str(),
                self.interactive_cgroup.as_deref(),
            ),
            (
                CgroupTargetRole::Background.as_str(),
                self.background_cgroup.as_deref(),
            ),
            (CgroupTargetRole::Game.as_str(), self.game_cgroup.as_deref()),
            (
                CgroupTargetRole::Compile.as_str(),
                self.compile_cgroup.as_deref(),
            ),
        ]
        .into_iter()
        .filter_map(|(name, target)| target.map(|target| (name, target)))
        .collect()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        for (name, target) in self.named_targets() {
            normalize_cgroup_target_path(target)
                .map(|_| ())
                .map_err(|err| anyhow::anyhow!("invalid {name}_cgroup target: {err}"))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CgroupTargetRole {
    Interactive,
    Background,
    Game,
    Compile,
}

impl CgroupTargetRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Background => "background",
            Self::Game => "game",
            Self::Compile => "compile",
        }
    }
}

pub fn normalize_cgroup_target_path(path: &Path) -> anyhow::Result<String> {
    if !path.is_absolute() {
        anyhow::bail!("cgroup target path must be absolute within the cgroup v2 namespace");
    }

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::bail!(
                    "cgroup target path must not contain parent traversal: {}",
                    path.display()
                )
            }
            Component::Prefix(_) => {
                anyhow::bail!(
                    "cgroup target path must not contain platform prefixes: {}",
                    path.display()
                )
            }
        }
    }

    if parts.is_empty() {
        anyhow::bail!("cgroup target path must not be the cgroup root");
    }

    Ok(format!("/{}", parts.join("/")))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonRetentionConfig {
    pub max_history_events: usize,
    pub max_state_snapshots: usize,
    pub retain_crash_diagnostics: bool,
}

impl Default for DaemonRetentionConfig {
    fn default() -> Self {
        Self {
            max_history_events: 10_000,
            max_state_snapshots: 16,
            retain_crash_diagnostics: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonRemoteConfig {
    pub allow_remote_apply: bool,
    pub require_auth_for_apply: bool,
    pub allow_non_loopback_apply: bool,
}

impl Default for DaemonRemoteConfig {
    fn default() -> Self {
        Self {
            allow_remote_apply: false,
            require_auth_for_apply: true,
            allow_non_loopback_apply: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonCandidateConfidenceConfig {
    pub min_suggest_confidence: f32,
    pub min_apply_low_risk_confidence: f32,
    pub min_apply_medium_risk_confidence: f32,
    pub min_high_risk_suggestion_confidence: f32,
}

impl Default for DaemonCandidateConfidenceConfig {
    fn default() -> Self {
        Self {
            min_suggest_confidence: 0.50,
            min_apply_low_risk_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
            min_apply_medium_risk_confidence: 0.85,
            min_high_risk_suggestion_confidence: 0.90,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonAutotuneConfig {
    pub candidate_window_seconds: u64,
    pub washout_seconds: u64,
    pub rollback_on_crash_recovery: bool,
    pub confidence: DaemonCandidateConfidenceConfig,
}

impl Default for DaemonAutotuneConfig {
    fn default() -> Self {
        Self {
            candidate_window_seconds: 30,
            washout_seconds: 10,
            rollback_on_crash_recovery: true,
            confidence: DaemonCandidateConfidenceConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_config_default_serializes() {
        let config = DaemonConfig::default();

        let json = serde_json::to_string(&config).unwrap();

        assert!(json.contains("\"mode\":\"observe\""));
        assert!(json.contains("\"preset\":\"observe-only\""));
        assert!(json.contains("\"source\":\"cli\""));
        assert!(json.contains("\"retain_crash_diagnostics\":true"));
    }

    #[test]
    fn daemon_config_owns_user_intent_fields() {
        let mut config = DaemonConfig {
            preset: DaemonPreset::GamingLowRisk,
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::RemoteAgent,
            ..DaemonConfig::default()
        };
        config.target.tree_pids.push(1234);
        config.target.require_explicit_target = true;
        config.safety.max_safety_class = SafetyClass::ReversibleLowRisk;
        config.safety.allow_system_wide_suggestions = true;
        config.safety.allow_system_wide_apply = false;
        config.retention.max_state_snapshots = 4;
        config.remote.allow_remote_apply = true;
        config.autotune.candidate_window_seconds = 60;

        assert_eq!(config.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(config.preset, DaemonPreset::GamingLowRisk);
        assert_eq!(config.source, ActionSource::RemoteAgent);
        assert_eq!(config.target.tree_pids, vec![1234]);
        assert!(config.target.require_explicit_target);
        assert_eq!(
            config.safety.max_safety_class,
            SafetyClass::ReversibleLowRisk
        );
        assert!(config.safety.allow_system_wide_suggestions);
        assert!(!config.safety.allow_system_wide_apply);
        assert_eq!(config.retention.max_state_snapshots, 4);
        assert!(config.remote.allow_remote_apply);
        assert_eq!(config.autotune.candidate_window_seconds, 60);
        assert_eq!(config.autotune.confidence.min_suggest_confidence, 0.50);
        assert_eq!(
            config.autotune.confidence.min_apply_low_risk_confidence,
            crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
        );
        assert_eq!(
            config.autotune.confidence.min_apply_medium_risk_confidence,
            0.85
        );
        assert_eq!(
            config
                .autotune
                .confidence
                .min_high_risk_suggestion_confidence,
            0.90
        );
    }

    #[test]
    fn daemon_presets_map_to_expected_safe_policy_defaults() {
        let observe = DaemonConfig::from_preset(DaemonPreset::ObserveOnly, ActionSource::Cli);
        assert_eq!(observe.mode, DaemonMode::Observe);
        assert_eq!(observe.safety.max_safety_class, SafetyClass::ObserveOnly);
        assert!(observe.safety.enabled_action_families.is_empty());

        let gaming = DaemonConfig::from_preset(DaemonPreset::GamingLowRisk, ActionSource::Cli);
        assert_eq!(gaming.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(
            gaming.safety.max_safety_class,
            SafetyClass::ReversibleLowRisk
        );
        assert!(
            gaming
                .safety
                .enabled_action_families
                .contains("cpu_affinity_profile")
        );
        assert!(!gaming.safety.allow_system_wide_suggestions);
        assert!(!gaming.safety.allow_system_wide_apply);
        assert!(gaming.safety.min_confidence >= 0.85);

        let laptop = DaemonConfig::from_preset(DaemonPreset::GamingLaptopSafe, ActionSource::Cli);
        assert_eq!(laptop.mode, DaemonMode::ApplyLowRisk);
        assert!(laptop.safety.min_confidence > gaming.safety.min_confidence);
        assert!(laptop.health.max_cpu_temp_celsius < gaming.health.max_cpu_temp_celsius);

        let debug = DaemonConfig::from_preset(DaemonPreset::DebugAggressive, ActionSource::Cli);
        assert_eq!(debug.mode, DaemonMode::ApplyMediumRisk);
        assert_eq!(
            debug.safety.max_safety_class,
            SafetyClass::ReversibleMediumRisk
        );
        assert!(!debug.safety.allow_high_risk);
        assert!(debug.safety.enabled_action_families.contains("uclamp"));
    }

    #[test]
    fn daemon_health_config_maps_to_system_health_thresholds() {
        let config = DaemonHealthConfig {
            max_cpu_temp_celsius: 80,
            max_gpu_temp_celsius: 81,
            min_disk_available_bytes: 1_000_000_000,
            max_memory_pressure_some_avg10_percent: 12.5,
        };

        let thresholds = config.thresholds();

        assert_eq!(thresholds.max_cpu_temp_millidegrees, 80_000);
        assert_eq!(thresholds.max_gpu_temp_millidegrees, 81_000);
        assert_eq!(thresholds.min_disk_available_bytes, 1_000_000_000);
        assert_eq!(
            thresholds.max_memory_pressure_some_avg10_millipercent,
            12_500
        );
    }

    #[test]
    fn daemon_preset_parser_accepts_documented_names() {
        assert_eq!(
            "observe-only".parse::<DaemonPreset>().unwrap(),
            DaemonPreset::ObserveOnly
        );
        assert_eq!(
            DaemonPreset::DebugAggressive.to_string(),
            "debug-aggressive"
        );
        assert!("risky".parse::<DaemonPreset>().is_err());
    }
}
