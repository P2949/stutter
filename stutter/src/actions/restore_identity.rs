use std::{fs, path::Path};

use crate::{actions::model::TaskRestoreIdentity, process::snapshot::parse_proc_stat_starttime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreIdentityStatus {
    SameTask,
    Missing,
    Mismatch { reason: String },
    UnknownLegacy,
}

pub fn verify_task_identity(
    proc_root: &Path,
    identity: &TaskRestoreIdentity,
) -> RestoreIdentityStatus {
    let tid = identity.tid;
    let stat_path = proc_root.join(tid.to_string()).join("stat");

    let Ok(stat_content) = fs::read_to_string(&stat_path) else {
        return RestoreIdentityStatus::Missing;
    };

    let current_starttime = parse_proc_stat_starttime(&stat_content);

    match (identity.starttime_ticks, current_starttime) {
        (Some(expected), Some(actual)) => {
            if expected != actual {
                return RestoreIdentityStatus::Mismatch {
                    reason: format!(
                        "starttime_ticks mismatch: expected={expected} actual={actual}"
                    ),
                };
            }
        }
        (None, _) => {
            return RestoreIdentityStatus::UnknownLegacy;
        }
        (_, None) => {
            return RestoreIdentityStatus::Mismatch {
                reason: "unable to parse current starttime".to_owned(),
            };
        }
    }

    if let Some(expected_process_pid) = identity.process_pid {
        match read_proc_status_tgid(proc_root, tid) {
            Ok(Some(actual_process_pid)) if actual_process_pid != expected_process_pid => {
                return RestoreIdentityStatus::Mismatch {
                    reason: format!(
                        "process_pid mismatch: expected={expected_process_pid} actual={actual_process_pid}"
                    ),
                };
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                return RestoreIdentityStatus::Mismatch {
                    reason: "unable to parse current process_pid".to_owned(),
                };
            }
            Err(_) => {
                return RestoreIdentityStatus::Mismatch {
                    reason: "unable to read current process_pid".to_owned(),
                };
            }
        }
    }

    if let (Some(process_pid), Some(expected_starttime)) =
        (identity.process_pid, identity.process_starttime_ticks)
    {
        let current_process_starttime = if process_pid == tid {
            current_starttime
        } else {
            let process_stat_path = proc_root.join(process_pid.to_string()).join("stat");
            fs::read_to_string(&process_stat_path)
                .ok()
                .and_then(|stat| parse_proc_stat_starttime(&stat))
        };

        match current_process_starttime {
            Some(actual_starttime) if actual_starttime != expected_starttime => {
                return RestoreIdentityStatus::Mismatch {
                    reason: format!(
                        "process_starttime_ticks mismatch: expected={expected_starttime} actual={actual_starttime}"
                    ),
                };
            }
            Some(_) => {}
            None => {
                return RestoreIdentityStatus::Mismatch {
                    reason: "unable to verify current process starttime".to_owned(),
                };
            }
        }
    }

    warn_if_comm_changed(proc_root, identity);
    warn_if_exe_changed(proc_root, identity);

    RestoreIdentityStatus::SameTask
}

fn read_proc_status_tgid(proc_root: &Path, tid: u32) -> std::io::Result<Option<u32>> {
    let status = fs::read_to_string(proc_root.join(tid.to_string()).join("status"))?;
    Ok(status.lines().find_map(|line| {
        let value = line.strip_prefix("Tgid:")?.trim();
        value.parse().ok()
    }))
}

fn warn_if_comm_changed(proc_root: &Path, identity: &TaskRestoreIdentity) {
    let Some(expected_comm) = identity.comm.as_deref() else {
        return;
    };

    let comm_path = proc_root.join(identity.tid.to_string()).join("comm");
    let Ok(current_comm) = fs::read_to_string(comm_path) else {
        return;
    };
    let current_comm = current_comm.trim();
    if !current_comm.is_empty() && current_comm != expected_comm {
        log::warn!(
            "restore identity advisory mismatch for tid={}: comm expected={:?} actual={:?}",
            identity.tid,
            expected_comm,
            current_comm
        );
    }
}

fn warn_if_exe_changed(proc_root: &Path, identity: &TaskRestoreIdentity) {
    let Some(expected_exe) = identity.exe.as_ref() else {
        return;
    };

    let exe_path = proc_root.join(identity.tid.to_string()).join("exe");
    let Ok(current_exe) = fs::read_link(exe_path) else {
        return;
    };
    if &current_exe != expected_exe {
        log::warn!(
            "restore identity advisory mismatch for tid={}: exe expected={} actual={}",
            identity.tid,
            expected_exe.display(),
            current_exe.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn temp_proc_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "stutter-restore-identity-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn fake_stat(tid: u32, comm: &str, starttime_ticks: u64) -> String {
        let mut fields = vec!["0".to_owned(); 20];
        fields[0] = "S".to_owned();
        fields[19] = starttime_ticks.to_string();

        format!("{tid} ({comm}) {}\n", fields.join(" "))
    }

    fn write_task(proc_root: &Path, tid: u32, tgid: u32, comm: &str, starttime_ticks: u64) {
        let task_dir = proc_root.join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("stat"), fake_stat(tid, comm, starttime_ticks)).unwrap();
        fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
        fs::write(
            task_dir.join("status"),
            format!("Name:\t{comm}\nTgid:\t{tgid}\nPid:\t{tid}\n"),
        )
        .unwrap();
    }

    fn identity(
        tid: u32,
        process_pid: Option<u32>,
        starttime_ticks: Option<u64>,
    ) -> TaskRestoreIdentity {
        TaskRestoreIdentity {
            tid,
            process_pid,
            starttime_ticks,
            comm: Some("game".to_owned()),
            exe: None,
            process_starttime_ticks: None,
        }
    }

    #[test]
    fn verifies_same_task_starttime_and_process_pid() {
        let proc_root = temp_proc_root("same-task");
        write_task(&proc_root, 42, 40, "game", 9001);

        let status = verify_task_identity(&proc_root, &identity(42, Some(40), Some(9001)));

        assert_eq!(status, RestoreIdentityStatus::SameTask);
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn missing_task_is_reported_before_legacy_policy() {
        let proc_root = temp_proc_root("missing");

        let status = verify_task_identity(&proc_root, &identity(42, None, None));

        assert_eq!(status, RestoreIdentityStatus::Missing);
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn legacy_record_without_starttime_restores_compatibly_when_task_exists() {
        let proc_root = temp_proc_root("legacy");
        write_task(&proc_root, 42, 40, "game", 9001);

        let status = verify_task_identity(&proc_root, &identity(42, None, None));

        assert_eq!(status, RestoreIdentityStatus::UnknownLegacy);
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn starttime_mismatch_reports_reused_tid() {
        let proc_root = temp_proc_root("starttime-mismatch");
        write_task(&proc_root, 42, 40, "game", 9002);

        let status = verify_task_identity(&proc_root, &identity(42, Some(40), Some(9001)));

        assert!(matches!(
            status,
            RestoreIdentityStatus::Mismatch { reason } if reason.contains("starttime_ticks mismatch")
        ));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn process_pid_mismatch_reports_reused_tid() {
        let proc_root = temp_proc_root("process-pid-mismatch");
        write_task(&proc_root, 42, 41, "game", 9001);

        let status = verify_task_identity(&proc_root, &identity(42, Some(40), Some(9001)));

        assert!(matches!(
            status,
            RestoreIdentityStatus::Mismatch { reason } if reason.contains("process_pid mismatch")
        ));
        fs::remove_dir_all(proc_root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn comm_and_exe_mismatches_are_advisory() {
        let proc_root = temp_proc_root("advisory");
        write_task(&proc_root, 42, 40, "renamed", 9001);
        std::os::unix::fs::symlink("/usr/bin/other-game", proc_root.join("42/exe")).unwrap();
        let mut identity = identity(42, Some(40), Some(9001));
        identity.exe = Some(PathBuf::from("/usr/bin/game"));

        let status = verify_task_identity(&proc_root, &identity);

        assert_eq!(status, RestoreIdentityStatus::SameTask);
        fs::remove_dir_all(proc_root).ok();
    }
}
