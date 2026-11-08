//! Exact attribution of an IRQ to the PCI device that owns it (package W9).
//!
//! A name in `/proc/interrupts` is a hint. The kernel also records which IRQ numbers a PCI device's
//! MSI/MSI-X vectors use, as files under `/sys/bus/pci/devices/<slot>/msi_irqs/`. That file is the
//! attribution: an IRQ with no such file is `Unattributed`, never guessed from its name, and the
//! PCI class code (not the name) says whether the owner is storage, network or display. This reads
//! only; whether anything may steer an IRQ is a separate, stricter decision.
use std::{collections::BTreeMap, fs, path::Path};

use serde::{Deserialize, Serialize};

/// Most PCI devices and IRQ vectors read.
const MAX_DEVICES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PciClass {
    Storage,
    Network,
    Display,
    Multimedia,
    Usb,
    Other,
}

impl PciClass {
    /// From the 24-bit class code in `<slot>/class` (`0x010802` is NVMe: base class `0x01`).
    fn from_code(code: u32) -> Self {
        match (code >> 16, code >> 8 & 0xff) {
            (0x01, _) => Self::Storage,
            (0x02, _) => Self::Network,
            (0x03, _) => Self::Display,
            (0x04, _) => Self::Multimedia,
            (0x0c, 0x03) => Self::Usb,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrqOwner {
    Pci {
        slot: String,
        class: PciClass,
    },
    /// No PCI device claims this IRQ number in sysfs.
    Unattributed,
}

/// The owning device of every IRQ number that some PCI device claims.
pub fn attribute_irqs(pci_root: &Path) -> BTreeMap<u32, IrqOwner> {
    let mut owners = BTreeMap::new();
    let Ok(devices) = fs::read_dir(pci_root) else {
        return owners;
    };
    for device in devices.flatten().take(MAX_DEVICES) {
        let Ok(slot) = device.file_name().into_string() else {
            continue;
        };
        let class = fs::read_to_string(device.path().join("class"))
            .ok()
            .and_then(|text| u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok())
            .map_or(PciClass::Other, PciClass::from_code);
        let Ok(vectors) = fs::read_dir(device.path().join("msi_irqs")) else {
            continue;
        };
        for vector in vectors.flatten().take(MAX_DEVICES) {
            let Some(irq) = vector
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            owners.insert(
                irq,
                IrqOwner::Pci {
                    slot: slot.clone(),
                    class,
                },
            );
        }
    }
    owners
}

impl IrqOwner {
    /// The owner of `irq`, `Unattributed` when sysfs does not claim it.
    pub fn of(owners: &BTreeMap<u32, IrqOwner>, irq: u32) -> Self {
        owners.get(&irq).cloned().unwrap_or(Self::Unattributed)
    }

    /// Whether steering this IRQ could affect storage: those stay out of any automatic path.
    pub fn is_storage_or_unknown(&self) -> bool {
        matches!(
            self,
            Self::Unattributed
                | Self::Pci {
                    class: PciClass::Storage,
                    ..
                }
        )
    }
}

/// The risk of steering an IRQ, never lower than the name-based risk: an IRQ whose owner is
/// storage, or that no PCI device claims, is high risk whatever its name suggests.
pub fn risk_with_attribution(
    name_risk: crate::actions::irq_affinity::IrqAffinityRisk,
    owner: &IrqOwner,
) -> crate::actions::irq_affinity::IrqAffinityRisk {
    if owner.is_storage_or_unknown() {
        crate::actions::irq_affinity::IrqAffinityRisk::HighRisk
    } else {
        name_risk
    }
}

/// The risk for `irq` on the real host: the name-based risk, raised by what sysfs says owns the
/// IRQ. In unit tests there is no real sysfs to consult, so the name-based risk stands.
pub fn attributed_risk(
    irq: u32,
    name_risk: crate::actions::irq_affinity::IrqAffinityRisk,
) -> crate::actions::irq_affinity::IrqAffinityRisk {
    #[cfg(test)]
    {
        let _ = irq;
        name_risk
    }
    #[cfg(not(test))]
    {
        let owners = attribute_irqs(Path::new("/sys/bus/pci/devices"));
        risk_with_attribution(name_risk, &IrqOwner::of(&owners, irq))
    }
}

#[cfg(test)]
#[path = "irq_attribution_tests.rs"]
mod tests;
