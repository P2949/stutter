use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::actions::model::{
    CgroupCpusetRestoreRecord, CgroupRestoreRecord, CpuPowerRestoreRecord, GpuPowerRestoreRecord,
    IoPrioRestoreRecord, IrqAffinityRestoreRecord, NiceRestoreRecord, UclampRestoreRecord,
    VmKnobRestoreRecord,
};

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
                tid: 55,
                process_pid: Some(50),
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
}
