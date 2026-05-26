use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::actions::model::{
    CgroupCpusetRestoreRecord, CgroupRestoreRecord, CpuPowerRestoreRecord, GpuPowerRestoreRecord,
    IoPrioRestoreRecord, IrqAffinityRestoreRecord, NiceRestoreRecord, UclampRestoreRecord,
    VmKnobRestoreRecord,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollbackTokenKindError {
    expected: &'static str,
    actual: &'static str,
}

impl RollbackTokenKindError {
    pub fn new(expected: &'static str, actual: &'static str) -> Self {
        Self { expected, actual }
    }

    pub fn expected(self) -> &'static str {
        self.expected
    }

    pub fn actual(self) -> &'static str {
        self.actual
    }
}

impl std::fmt::Display for RollbackTokenKindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid rollback token: expected {}, actual {}",
            self.expected, self.actual
        )
    }
}

impl std::error::Error for RollbackTokenKindError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind")]
pub enum RollbackToken {
    #[serde(rename = "cpu-affinity-restore-file")]
    CpuAffinityRestoreFile {
        #[serde(rename = "restore_path")]
        path: PathBuf,
        affected_tasks: usize,
    },
    NiceRestore {
        records: Vec<NiceRestoreRecord>,
    },
    IrqAffinityRestore {
        records: Vec<IrqAffinityRestoreRecord>,
    },
    IoPrioRestore {
        records: Vec<IoPrioRestoreRecord>,
    },
    UclampRestore {
        records: Vec<UclampRestoreRecord>,
    },
    CgroupRestore {
        records: Vec<CgroupRestoreRecord>,
        #[serde(default)]
        cpuset: Option<CgroupCpusetRestoreRecord>,
    },
    CpuPowerRestore {
        records: Vec<CpuPowerRestoreRecord>,
    },
    VmKnobRestore {
        records: Vec<VmKnobRestoreRecord>,
    },
    GpuPowerRestore {
        records: Vec<GpuPowerRestoreRecord>,
    },
    SysfsRestore {
        path: PathBuf,
        original_value: String,
    },
}

impl RollbackToken {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::CpuAffinityRestoreFile { .. } => "cpu-affinity-restore-file",
            Self::NiceRestore { .. } => "nice-restore",
            Self::IrqAffinityRestore { .. } => "irq-affinity-restore",
            Self::IoPrioRestore { .. } => "ioprio-restore",
            Self::UclampRestore { .. } => "uclamp-restore",
            Self::CgroupRestore { .. } => "cgroup-restore",
            Self::CpuPowerRestore { .. } => "cpu-power-restore",
            Self::VmKnobRestore { .. } => "vm-knob-restore",
            Self::GpuPowerRestore { .. } => "gpu-power-restore",
            Self::SysfsRestore { .. } => "sysfs-restore",
        }
    }

    pub fn kind_error(&self, expected: &'static str) -> RollbackTokenKindError {
        RollbackTokenKindError::new(expected, self.kind())
    }

    pub fn as_cpu_affinity_restore_file(&self) -> Option<(&Path, usize)> {
        match self {
            Self::CpuAffinityRestoreFile {
                path,
                affected_tasks,
            } => Some((path.as_path(), *affected_tasks)),
            _ => None,
        }
    }

    pub fn into_cpu_affinity_restore_file(
        self,
    ) -> Result<(PathBuf, usize), RollbackTokenKindError> {
        match self {
            Self::CpuAffinityRestoreFile {
                path,
                affected_tasks,
            } => Ok((path, affected_tasks)),
            other => Err(other.kind_error("cpu-affinity-restore-file")),
        }
    }

    pub fn as_nice_restore(&self) -> Option<&[NiceRestoreRecord]> {
        match self {
            Self::NiceRestore { records } => Some(records),
            _ => None,
        }
    }

    pub fn into_nice_restore(self) -> Result<Vec<NiceRestoreRecord>, RollbackTokenKindError> {
        match self {
            Self::NiceRestore { records } => Ok(records),
            other => Err(other.kind_error("nice-restore")),
        }
    }

    pub fn as_irq_affinity_restore(&self) -> Option<&[IrqAffinityRestoreRecord]> {
        match self {
            Self::IrqAffinityRestore { records } => Some(records),
            _ => None,
        }
    }

    pub fn into_irq_affinity_restore(
        self,
    ) -> Result<Vec<IrqAffinityRestoreRecord>, RollbackTokenKindError> {
        match self {
            Self::IrqAffinityRestore { records } => Ok(records),
            other => Err(other.kind_error("irq-affinity-restore")),
        }
    }

    pub fn as_ioprio_restore(&self) -> Option<&[IoPrioRestoreRecord]> {
        match self {
            Self::IoPrioRestore { records } => Some(records),
            _ => None,
        }
    }

    pub fn into_ioprio_restore(self) -> Result<Vec<IoPrioRestoreRecord>, RollbackTokenKindError> {
        match self {
            Self::IoPrioRestore { records } => Ok(records),
            other => Err(other.kind_error("ioprio-restore")),
        }
    }

    pub fn as_uclamp_restore(&self) -> Option<&[UclampRestoreRecord]> {
        match self {
            Self::UclampRestore { records } => Some(records),
            _ => None,
        }
    }

    pub fn into_uclamp_restore(self) -> Result<Vec<UclampRestoreRecord>, RollbackTokenKindError> {
        match self {
            Self::UclampRestore { records } => Ok(records),
            other => Err(other.kind_error("uclamp-restore")),
        }
    }

    pub fn as_cgroup_restore(
        &self,
    ) -> Option<(&[CgroupRestoreRecord], Option<&CgroupCpusetRestoreRecord>)> {
        match self {
            Self::CgroupRestore { records, cpuset } => Some((records, cpuset.as_ref())),
            _ => None,
        }
    }

    pub fn into_cgroup_restore(
        self,
    ) -> Result<(Vec<CgroupRestoreRecord>, Option<CgroupCpusetRestoreRecord>), RollbackTokenKindError>
    {
        match self {
            Self::CgroupRestore { records, cpuset } => Ok((records, cpuset)),
            other => Err(other.kind_error("cgroup-restore")),
        }
    }

    pub fn as_cpu_power_restore(&self) -> Option<&[CpuPowerRestoreRecord]> {
        match self {
            Self::CpuPowerRestore { records } => Some(records),
            _ => None,
        }
    }

    pub fn into_cpu_power_restore(
        self,
    ) -> Result<Vec<CpuPowerRestoreRecord>, RollbackTokenKindError> {
        match self {
            Self::CpuPowerRestore { records } => Ok(records),
            other => Err(other.kind_error("cpu-power-restore")),
        }
    }

    pub fn as_vm_knob_restore(&self) -> Option<&[VmKnobRestoreRecord]> {
        match self {
            Self::VmKnobRestore { records } => Some(records),
            _ => None,
        }
    }

    pub fn into_vm_knob_restore(self) -> Result<Vec<VmKnobRestoreRecord>, RollbackTokenKindError> {
        match self {
            Self::VmKnobRestore { records } => Ok(records),
            other => Err(other.kind_error("vm-knob-restore")),
        }
    }

    pub fn as_gpu_power_restore(&self) -> Option<&[GpuPowerRestoreRecord]> {
        match self {
            Self::GpuPowerRestore { records } => Some(records),
            _ => None,
        }
    }

    pub fn into_gpu_power_restore(
        self,
    ) -> Result<Vec<GpuPowerRestoreRecord>, RollbackTokenKindError> {
        match self {
            Self::GpuPowerRestore { records } => Ok(records),
            other => Err(other.kind_error("gpu-power-restore")),
        }
    }

    pub fn as_sysfs_restore(&self) -> Option<(&Path, &str)> {
        match self {
            Self::SysfsRestore {
                path,
                original_value,
            } => Some((path.as_path(), original_value)),
            _ => None,
        }
    }

    pub fn into_sysfs_restore(self) -> Result<(PathBuf, String), RollbackTokenKindError> {
        match self {
            Self::SysfsRestore {
                path,
                original_value,
            } => Ok((path, original_value)),
            other => Err(other.kind_error("sysfs-restore")),
        }
    }

    pub fn affected_tasks(&self) -> usize {
        match self {
            Self::CpuAffinityRestoreFile { affected_tasks, .. } => *affected_tasks,
            Self::NiceRestore { records } => records.len(),
            Self::IrqAffinityRestore { records } => records.len(),
            Self::IoPrioRestore { records } => records.len(),
            Self::UclampRestore { records } => records.len(),
            Self::CgroupRestore { records, .. } => records.len(),
            Self::CpuPowerRestore { records } => records.len(),
            Self::VmKnobRestore { records } => records.len(),
            Self::GpuPowerRestore { records } => records.len(),
            Self::SysfsRestore { .. } => 1,
        }
    }

    pub fn restore_path(&self) -> Option<&PathBuf> {
        match self {
            Self::CpuAffinityRestoreFile { path, .. } => Some(path),
            Self::SysfsRestore { path, .. } => Some(path),
            Self::NiceRestore { .. }
            | Self::IrqAffinityRestore { .. }
            | Self::IoPrioRestore { .. }
            | Self::UclampRestore { .. }
            | Self::CgroupRestore { .. }
            | Self::CpuPowerRestore { .. }
            | Self::GpuPowerRestore { .. }
            | Self::VmKnobRestore { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::actions::TaskRestoreIdentity;

    #[test]
    fn legacy_task_restore_tokens_without_identity_deserialize() {
        let nice: RollbackToken = serde_json::from_value(json!({
            "kind": "NiceRestore",
            "records": [{ "tid": 42, "original_nice": 5 }]
        }))
        .unwrap();
        let RollbackToken::NiceRestore { records } = nice else {
            panic!("unexpected token kind");
        };
        assert_eq!(records[0].tid(), 42);
        assert_eq!(records[0].identity, None);
        assert_eq!(records[0].restore_identity().starttime_ticks, None);

        let ioprio: RollbackToken = serde_json::from_value(json!({
            "kind": "IoPrioRestore",
            "records": [{ "tid": 43, "original_ioprio": 16384 }]
        }))
        .unwrap();
        let RollbackToken::IoPrioRestore { records } = ioprio else {
            panic!("unexpected token kind");
        };
        assert_eq!(records[0].tid(), 43);
        assert_eq!(records[0].identity, None);

        let uclamp: RollbackToken = serde_json::from_value(json!({
            "kind": "UclampRestore",
            "records": [{
                "tid": 44,
                "original_util_min": 0,
                "original_util_max": 1024
            }]
        }))
        .unwrap();
        let RollbackToken::UclampRestore { records } = uclamp else {
            panic!("unexpected token kind");
        };
        assert_eq!(records[0].tid(), 44);
        assert_eq!(records[0].identity, None);

        let cgroup: RollbackToken = serde_json::from_value(json!({
            "kind": "CgroupRestore",
            "records": [{
                "tid": 45,
                "original_cgroup": "/sys/fs/cgroup/game.slice"
            }]
        }))
        .unwrap();
        let RollbackToken::CgroupRestore { records, .. } = cgroup else {
            panic!("unexpected token kind");
        };
        assert_eq!(records[0].tid(), 45);
        assert_eq!(records[0].identity, None);
        assert_eq!(
            records[0].original_cgroup,
            PathBuf::from("/sys/fs/cgroup/game.slice")
        );
    }

    #[test]
    fn identity_bearing_restore_tokens_deserialize_old_and_new_identity_fields() {
        let token: RollbackToken = serde_json::from_value(json!({
            "kind": "NiceRestore",
            "records": [{
                "original_nice": 0,
                "identity": {
                    "tid": 55,
                    "process_pid": 50,
                    "task_starttime_ticks": 9001,
                    "comm": "game-main",
                    "exe": "/usr/bin/game"
                }
            }]
        }))
        .unwrap();

        let RollbackToken::NiceRestore { records } = token else {
            panic!("unexpected token kind");
        };
        assert_eq!(records[0].tid(), 55);
        assert_eq!(records[0].tid, 0);
        assert_eq!(
            records[0].identity,
            Some(TaskRestoreIdentity {
                tid: (55).into(),
                process_pid: Some((50).into()),
                starttime_ticks: Some(9001),
                comm: Some("game-main".to_owned()),
                exe: Some(PathBuf::from("/usr/bin/game")),
                process_starttime_ticks: None,
            })
        );
    }

    #[test]
    fn new_restore_tokens_serialize_top_level_tid_and_identity() {
        let token = RollbackToken::NiceRestore {
            records: vec![NiceRestoreRecord::new(
                TaskRestoreIdentity::observed(
                    66,
                    Some(60),
                    Some("render".to_owned()),
                    Some(1234),
                    Some(PathBuf::from("/usr/bin/render")),
                ),
                10,
            )],
        };

        let value = serde_json::to_value(&token).unwrap();

        assert_eq!(value["records"][0]["tid"], 66);
        assert_eq!(value["records"][0]["identity"]["tid"], 66);
        assert_eq!(value["records"][0]["identity"]["process_pid"], 60);
        assert_eq!(value["records"][0]["identity"]["starttime_ticks"], 1234);
        assert_eq!(value["records"][0]["identity"]["comm"], "render");
        assert_eq!(value["records"][0]["identity"]["exe"], "/usr/bin/render");
    }

    #[test]
    fn typed_restore_accessors_report_expected_and_actual_kind() {
        let token = RollbackToken::IrqAffinityRestore {
            records: vec![IrqAffinityRestoreRecord {
                irq: 44,
                device_hint: "amdgpu".to_owned(),
                original_smp_affinity: "00000001".to_owned(),
            }],
        };

        assert_eq!(token.as_irq_affinity_restore().unwrap()[0].irq, 44);
        assert!(token.as_nice_restore().is_none());

        let error = token.clone().into_nice_restore().unwrap_err();
        assert_eq!(error.expected(), "nice-restore");
        assert_eq!(error.actual(), "irq-affinity-restore");

        let records = token.into_irq_affinity_restore().unwrap();
        assert_eq!(records[0].device_hint, "amdgpu");
    }
}
