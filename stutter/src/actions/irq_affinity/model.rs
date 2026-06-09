use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrqAffinityRisk {
    ReversibleMediumRisk,
    HighRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrqAffinityEvidence {
    pub strong_irq_evidence: bool,
    pub stable_irq_identity: bool,
    pub known_device_mapping: bool,
    pub observed_irq: Option<u32>,
    pub observed_device_hint: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct IrqAffinityPolicy {
    pub allow_irq_affinity_changes: bool,
    pub allow_high_risk_devices: bool,
    pub require_strong_irq_evidence: bool,
    pub require_stable_irq_identity: bool,
    pub require_known_device_mapping: bool,
}

impl Default for IrqAffinityPolicy {
    fn default() -> Self {
        Self {
            allow_irq_affinity_changes: false,
            allow_high_risk_devices: false,
            require_strong_irq_evidence: true,
            require_stable_irq_identity: true,
            require_known_device_mapping: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrqAffinityAction {
    pub irq: u32,
    pub device_hint: String,
    pub smp_affinity: String,
    pub risk: IrqAffinityRisk,
    pub evidence: IrqAffinityEvidence,
    pub irq_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IrqAffinitySnapshot {
    pub(super) irq: u32,
    pub(super) device_hint: String,
    pub(super) smp_affinity: String,
}

impl IrqAffinityAction {
    pub fn new(
        irq: u32,
        device_hint: String,
        smp_affinity: String,
        risk: IrqAffinityRisk,
        evidence: IrqAffinityEvidence,
    ) -> Self {
        Self {
            irq,
            device_hint,
            smp_affinity,
            risk,
            evidence,
            irq_root: PathBuf::from("/proc/irq"),
        }
    }
}
