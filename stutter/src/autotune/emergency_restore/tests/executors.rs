use std::{fs, path::PathBuf};

use super::{super::executors::*, support::*};
use crate::actions::*;

#[test]
fn restore_task_records_skips_esrch_and_continues() {
    let records = [1_u32, 2_u32, 3_u32];

    let summary = restore_task_records(
        "test-restore",
        &records,
        |record| match *record {
            1 => Ok(()),
            2 => Err(std::io::Error::from_raw_os_error(libc::ESRCH)),
            3 => Ok(()),
            _ => unreachable!(),
        },
        |record| format!("failed record={record}"),
    )
    .unwrap();

    assert_eq!(summary.rollback_kind, "test-restore");
    assert_eq!(summary.restored_items, 2);
    assert_eq!(summary.skipped_items, 1);
    assert_eq!(summary.skipped_missing, 1);
    assert_eq!(summary.skipped_identity_mismatch, 0);
    assert_eq!(summary.failed_items, 0);
    assert!(
        summary
            .messages
            .iter()
            .any(|message| message.contains("skipped_missing_tasks=1"))
    );
}

#[test]
fn restore_task_records_returns_error_for_non_esrch() {
    let records = [1_u32, 2_u32, 3_u32];

    let err = restore_task_records(
        "test-restore",
        &records,
        |record| match *record {
            1 => Ok(()),
            2 => Err(std::io::Error::from_raw_os_error(libc::EPERM)),
            3 => panic!("must not continue after non-ESRCH failure"),
            _ => unreachable!(),
        },
        |record| format!("failed record={record}"),
    )
    .unwrap_err();

    let message = format!("{err:#}");
    assert!(message.contains("failed record=2"));
}

#[test]
fn restore_task_records_empty_list_succeeds() {
    let records: [u32; 0] = [];

    let summary = restore_task_records(
        "test-restore",
        &records,
        |_| Ok(()),
        |record| format!("failed record={record}"),
    )
    .unwrap();

    assert_eq!(summary.restored_items, 0);
    assert_eq!(summary.skipped_items, 0);
    assert_eq!(summary.skipped_missing, 0);
    assert_eq!(summary.failed_items, 0);
}

#[test]
fn restore_rollback_token_supports_all_sysfs_record_collections() {
    let dir = temp_dir("record-collections");
    let cpu_path = dir.join("cpu");
    let vm_path = dir.join("vm");
    let gpu_path = dir.join("gpu");
    fs::write(&cpu_path, "bad").unwrap();
    fs::write(&vm_path, "bad").unwrap();
    fs::write(&gpu_path, "bad").unwrap();

    restore_rollback_token(&RollbackToken::CpuPowerRestore {
        records: vec![CpuPowerRestoreRecord {
            path: cpu_path.clone(),
            original_value: "cpu-original".to_owned(),
        }],
    })
    .unwrap();
    restore_rollback_token(&RollbackToken::VmKnobRestore {
        records: vec![VmKnobRestoreRecord {
            path: vm_path.clone(),
            original_value: "vm-original".to_owned(),
        }],
    })
    .unwrap();
    restore_rollback_token(&RollbackToken::GpuPowerRestore {
        records: vec![GpuPowerRestoreRecord {
            path: gpu_path.clone(),
            original_value: "gpu-original".to_owned(),
        }],
    })
    .unwrap();

    assert_eq!(fs::read_to_string(cpu_path).unwrap(), "cpu-original");
    assert_eq!(fs::read_to_string(vm_path).unwrap(), "vm-original");
    assert_eq!(fs::read_to_string(gpu_path).unwrap(), "gpu-original");

    fs::remove_dir_all(dir).ok();
}

#[test]
fn direct_cgroup_restore_resolves_namespace_absolute_paths_under_root() {
    let dir = temp_dir("direct-cgroup-records");
    let cgroup_root = dir.join("cgroup");
    let old_cgroup = cgroup_root.join("old.slice");
    fs::create_dir_all(&old_cgroup).unwrap();
    fs::write(old_cgroup.join("cgroup.procs"), "").unwrap();

    let records = vec![CgroupRestoreRecord::new(
        TaskRestoreIdentity::observed(42, None, Some("game-thread".to_owned()), None, None),
        PathBuf::from("/old.slice"),
    )];

    let summary = restore_cgroup_records_at(&cgroup_root, &records).unwrap();

    assert_eq!(summary.restored_items, 1);
    assert_eq!(
        fs::read_to_string(old_cgroup.join("cgroup.procs")).unwrap(),
        "42"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn direct_cgroup_cpuset_restore_resolves_namespace_absolute_paths_under_root() {
    let dir = temp_dir("direct-cgroup-cpuset");
    let cgroup_root = dir.join("cgroup");
    let target_cgroup = cgroup_root.join("stutter/game.slice");
    fs::create_dir_all(&target_cgroup).unwrap();
    fs::write(target_cgroup.join("cpuset.cpus"), "2-3").unwrap();
    fs::write(target_cgroup.join("cpuset.mems"), "1").unwrap();

    let record = CgroupCpusetRestoreRecord {
        cgroup_path: PathBuf::from("/stutter/game.slice"),
        original_cpuset_cpus: Some("0-7".to_owned()),
        original_cpuset_mems: Some("0".to_owned()),
    };

    let restored = restore_cgroup_cpuset_record_at(&cgroup_root, &record).unwrap();

    assert_eq!(restored, 2);
    assert_eq!(
        fs::read_to_string(target_cgroup.join("cpuset.cpus")).unwrap(),
        "0-7"
    );
    assert_eq!(
        fs::read_to_string(target_cgroup.join("cpuset.mems")).unwrap(),
        "0"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn irq_affinity_restore_skips_when_irq_identity_changes() {
    let dir = temp_dir("irq-skip");
    let irq_root = dir.join("proc_irq");
    let irq_dir = irq_root.join("44");
    fs::create_dir_all(&irq_dir).unwrap();
    fs::write(irq_dir.join("actions"), "nvme\n").unwrap();
    fs::write(irq_dir.join("smp_affinity"), "00000002\n").unwrap();

    let records = vec![IrqAffinityRestoreRecord {
        irq: 44,
        device_hint: "amdgpu".to_owned(),
        original_smp_affinity: "00000001".to_owned(),
    }];

    let summary = restore_irq_affinity_records_at(&irq_root, &records).unwrap();

    assert_eq!(summary.restored_items, 0);
    assert_eq!(summary.skipped_items, 1);
    assert!(summary.messages[0].contains("device mapping changed"));
    assert_eq!(
        fs::read_to_string(irq_dir.join("smp_affinity")).unwrap(),
        "00000002\n"
    );

    fs::remove_dir_all(dir).ok();
}
