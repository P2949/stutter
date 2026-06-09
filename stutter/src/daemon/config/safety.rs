use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Serialize};

use super::*;
use crate::actions::SafetyClass;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonSafetyConfig {
    pub max_safety_class: SafetyClass,
    pub allowed_action_classes: BTreeSet<SafetyClass>,
    pub enabled_action_families: BTreeSet<String>,
    pub denied_action_families: BTreeSet<String>,
    pub cgroup_targets: DaemonCgroupTargetsConfig,
    #[serde(default)]
    pub system_wide_allowlist: DaemonSystemWideAllowlistConfig,
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
            system_wide_allowlist: DaemonSystemWideAllowlistConfig::default(),
            allow_system_wide_suggestions: false,
            allow_system_wide_apply: false,
            allow_high_risk: false,
            allow_persistent_effects: false,
            min_confidence: 0.0,
        }
    }
}

pub(crate) fn safety_classes_up_to(max: SafetyClass) -> BTreeSet<SafetyClass> {
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
pub struct DaemonSystemWideAllowlistConfig {
    pub cpu_policies: BTreeSet<String>,
    pub gpu_cards: BTreeSet<String>,
    pub gpu_pci_ids: BTreeSet<String>,
    pub irq_devices: BTreeSet<String>,
    pub vm_knobs: BTreeSet<String>,
}

impl DaemonSystemWideAllowlistConfig {
    pub fn allows_gpu(&self, card: &str, pci_id: Option<&str>) -> bool {
        self.gpu_cards.contains(card)
            || pci_id.is_some_and(|pci_id| {
                self.gpu_pci_ids
                    .iter()
                    .any(|allowed| wildcard_match(allowed, pci_id))
            })
    }

    pub fn allows_irq_device(&self, device: &str) -> bool {
        let device = device.to_ascii_lowercase();
        self.irq_devices
            .iter()
            .any(|allowed| device.contains(&allowed.to_ascii_lowercase()))
    }

    pub fn allows_vm_knob(&self, path: &Path) -> bool {
        let normalized = normalize_vm_knob_path(path);
        self.vm_knobs
            .iter()
            .any(|allowed| normalize_vm_knob_text(allowed) == normalized)
    }
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return value.starts_with(prefix);
    }
    pattern == value
}

fn normalize_vm_knob_path(path: &Path) -> String {
    normalize_vm_knob_text(&path.to_string_lossy())
}

fn normalize_vm_knob_text(path: &str) -> String {
    path.trim()
        .trim_start_matches('/')
        .strip_prefix("proc/sys/")
        .unwrap_or_else(|| path.trim().trim_start_matches('/'))
        .replace('.', "/")
}
