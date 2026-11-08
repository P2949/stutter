//! Typed cgroup resource-control values and their preflight (package W11a).
//!
//! Each knob is its own type with its own bounds, risk and kernel semantics. This is the value
//! model and the read-only preflight; it performs no write. A write path must prepare, journal,
//! commit, read back and restore through the existing transaction machinery, and no operation here
//! inherits the maturity of cgroup *placement*.
use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

/// One controllable attribute with a bounded value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CgroupControl {
    CpuWeight(u32),
    CpuIdle(bool),
    /// `quota_us` of `period_us`; `None` quota means unlimited (`max`).
    CpuMax {
        quota_us: Option<u64>,
        period_us: u64,
    },
    CpusetCpus(Vec<u32>),
    CpusetMems(Vec<u32>),
    MemoryMin(u64),
    MemoryLow(u64),
    /// `None` means unlimited (`max`).
    MemoryHigh(Option<u64>),
    IoWeight(u32),
}

/// How cautiously a control must be treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ControlRisk {
    /// Relative share only: weight, idle.
    RelativeShare,
    /// Placement limits: cpuset.
    Placement,
    /// A hard throttle or a reclaim protection with global effect: `cpu.max`, `memory.min`,
    /// `memory.high`. Needs stricter guardrails.
    HardLimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlDenial {
    /// The value is outside the kernel's range.
    OutOfRange(&'static str),
    /// The controller is not enabled for this cgroup by its parent's `subtree_control`.
    ControllerNotEnabled(&'static str),
    /// The cgroup is a threaded domain, where these controls are not valid.
    ThreadedGroup,
    /// `cpu.weight` is not honored while `cpu.idle` is set on the same group.
    IdleOverridesWeight,
    /// The requested CPUs or nodes are not in the effective set the parent allows.
    NotInEffectiveSet,
    /// No device honors `io.weight` on this host.
    IoWeightNotHonored,
    /// The cgroup is not delegated to this user.
    NotDelegated,
}

impl CgroupControl {
    /// The file this control writes and the controller it needs.
    pub fn file_and_controller(&self) -> (&'static str, &'static str) {
        match self {
            Self::CpuWeight(_) => ("cpu.weight", "cpu"),
            Self::CpuIdle(_) => ("cpu.idle", "cpu"),
            Self::CpuMax { .. } => ("cpu.max", "cpu"),
            Self::CpusetCpus(_) => ("cpuset.cpus", "cpuset"),
            Self::CpusetMems(_) => ("cpuset.mems", "cpuset"),
            Self::MemoryMin(_) => ("memory.min", "memory"),
            Self::MemoryLow(_) => ("memory.low", "memory"),
            Self::MemoryHigh(_) => ("memory.high", "memory"),
            Self::IoWeight(_) => ("io.weight", "io"),
        }
    }

    pub fn risk(&self) -> ControlRisk {
        match self {
            Self::CpuWeight(_) | Self::CpuIdle(_) | Self::IoWeight(_) => ControlRisk::RelativeShare,
            Self::CpusetCpus(_) | Self::CpusetMems(_) | Self::MemoryLow(_) => {
                ControlRisk::Placement
            }
            Self::CpuMax { .. } | Self::MemoryMin(_) | Self::MemoryHigh(_) => {
                ControlRisk::HardLimit
            }
        }
    }

    /// The text the kernel expects in the control file.
    pub fn render(&self) -> String {
        let list = |ids: &[u32]| ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        match self {
            Self::CpuWeight(weight) | Self::IoWeight(weight) => weight.to_string(),
            Self::CpuIdle(idle) => u8::from(*idle).to_string(),
            Self::CpuMax {
                quota_us,
                period_us,
            } => match quota_us {
                Some(quota) => format!("{quota} {period_us}"),
                None => format!("max {period_us}"),
            },
            Self::CpusetCpus(cpus) => list(cpus),
            Self::CpusetMems(nodes) => list(nodes),
            Self::MemoryMin(bytes) | Self::MemoryLow(bytes) => bytes.to_string(),
            Self::MemoryHigh(bytes) => bytes.map_or_else(|| "max".to_owned(), |b| b.to_string()),
        }
    }

    /// Whether `current` (the file's contents) is exactly this control: the readback check.
    pub fn readback_matches(&self, current: &str) -> bool {
        let current = current.trim();
        match self {
            Self::CpuWeight(_) | Self::IoWeight(_) | Self::CpuIdle(_) => current == self.render(),
            // `io.weight` also lists per-device lines; the default line is `default <n>`.
            Self::CpuMax { .. } => current == self.render(),
            Self::CpusetCpus(_) | Self::CpusetMems(_) => crate::topology::parse_cpu_list(current)
                .map(crate::topology::sorted_unique)
                .is_ok_and(|parsed| match self {
                    Self::CpusetCpus(wanted) | Self::CpusetMems(wanted) => {
                        parsed == crate::topology::sorted_unique(wanted.clone())
                    }
                    _ => false,
                }),
            Self::MemoryMin(_) | Self::MemoryLow(_) | Self::MemoryHigh(_) => {
                current == self.render()
            }
        }
    }

    /// Range checks that need no host facts.
    pub fn validate_value(&self) -> Result<(), ControlDenial> {
        match self {
            Self::CpuWeight(weight) | Self::IoWeight(weight) if !(1..=10_000).contains(weight) => {
                Err(ControlDenial::OutOfRange("weight must be 1..=10000"))
            }
            Self::CpuMax {
                quota_us: Some(quota),
                period_us,
            } => {
                if !(1_000..=1_000_000).contains(period_us) {
                    Err(ControlDenial::OutOfRange(
                        "period must be 1000..=1000000 us",
                    ))
                } else if *quota < 1_000 {
                    Err(ControlDenial::OutOfRange("quota must be at least 1000 us"))
                } else {
                    Ok(())
                }
            }
            Self::CpuMax {
                quota_us: None,
                period_us,
            } if !(1_000..=1_000_000).contains(period_us) => Err(ControlDenial::OutOfRange(
                "period must be 1000..=1000000 us",
            )),
            Self::CpusetCpus(ids) | Self::CpusetMems(ids) if ids.is_empty() => {
                Err(ControlDenial::OutOfRange("a cpuset may not be empty"))
            }
            _ => Ok(()),
        }
    }
}

/// Host facts about one cgroup, read from its directory and its parent's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CgroupContext {
    /// Controllers the parent enables for this group (`cgroup.subtree_control` of the parent).
    pub enabled_controllers: Vec<String>,
    pub threaded: bool,
    pub cpu_idle: Option<bool>,
    pub effective_cpus: Option<Vec<u32>>,
    pub effective_mems: Option<Vec<u32>>,
    /// Whether any block device accepts `io.weight`; `None` when unknown.
    pub io_weight_honored: Option<bool>,
    /// The directory is writable by this user (delegation).
    pub delegated: bool,
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

impl CgroupContext {
    pub fn read(dir: &Path, euid: u32) -> Self {
        let parent = dir.parent().unwrap_or(dir);
        let list = |name: &str| {
            read(&dir.join(name))
                .and_then(|text| crate::topology::parse_cpu_list(&text).ok())
                .map(crate::topology::sorted_unique)
        };
        let owned =
            fs::metadata(dir).is_ok_and(|meta| std::os::unix::fs::MetadataExt::uid(&meta) == euid);
        Self {
            enabled_controllers: read(&parent.join("cgroup.subtree_control"))
                .map(|text| text.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
            threaded: read(&dir.join("cgroup.type"))
                .is_some_and(|kind| kind.starts_with("domain threaded") || kind == "threaded"),
            cpu_idle: read(&dir.join("cpu.idle")).map(|text| text == "1"),
            effective_cpus: list("cpuset.cpus.effective"),
            effective_mems: list("cpuset.mems.effective"),
            io_weight_honored: read(&dir.join("io.weight")).map(|text| !text.is_empty()),
            delegated: owned,
        }
    }

    /// The full read-only preflight of `control` for this cgroup.
    pub fn preflight(&self, control: &CgroupControl) -> Result<(), ControlDenial> {
        control.validate_value()?;
        if !self.delegated {
            return Err(ControlDenial::NotDelegated);
        }
        let (_, controller) = control.file_and_controller();
        if !self
            .enabled_controllers
            .iter()
            .any(|enabled| enabled == controller)
        {
            return Err(ControlDenial::ControllerNotEnabled(controller));
        }
        if self.threaded {
            return Err(ControlDenial::ThreadedGroup);
        }
        match control {
            CgroupControl::CpuWeight(_) if self.cpu_idle == Some(true) => {
                return Err(ControlDenial::IdleOverridesWeight);
            }
            CgroupControl::CpusetCpus(wanted) => {
                if let Some(effective) = &self.effective_cpus
                    && !wanted.iter().all(|cpu| effective.contains(cpu))
                {
                    return Err(ControlDenial::NotInEffectiveSet);
                }
            }
            CgroupControl::CpusetMems(wanted) => {
                if let Some(effective) = &self.effective_mems
                    && !wanted.iter().all(|node| effective.contains(node))
                {
                    return Err(ControlDenial::NotInEffectiveSet);
                }
            }
            CgroupControl::IoWeight(_) if self.io_weight_honored != Some(true) => {
                return Err(ControlDenial::IoWeightNotHonored);
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
