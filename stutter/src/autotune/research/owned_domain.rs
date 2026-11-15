//! The study-prefix ownership domain of an attempt: every process of this user whose own
//! environment names the attempt's private study compatdata. Steam never gives that path to
//! anything but the research dispatcher's Proton (the dispatcher substitutes it), and Wine and the
//! Steam Runtime reaper inherit it, so these processes remain attributable when the dispatcher has
//! exited or the game has not started yet. A process whose environment cannot be read is not
//! claimed, so this can only widen cleanup to processes that positively carry the study path.

use std::{os::unix::fs::MetadataExt, path::Path, time::Duration};

use stutter_core::ids::Pid;

use super::owned_cleanup::{OwnedProcess, terminate_exact};

const MAX_ENVIRON: u64 = 1 << 20;

fn start_ticks(proc_root: &Path, pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(proc_root.join(pid.to_string()).join("stat")).ok()?;
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(19)?.parse().ok()
}

fn carries_study_path(proc_root: &Path, pid: u32, study_compat_data: &Path) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(proc_root.join(pid.to_string()).join("environ")) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file.take(MAX_ENVIRON).read_to_end(&mut bytes).is_err() {
        return false;
    }
    let wanted = study_compat_data.as_os_str().as_encoded_bytes();
    bytes.split(|b| *b == 0).any(|entry| {
        entry
            .strip_prefix(b"STEAM_COMPAT_DATA_PATH=")
            .is_some_and(|value| value == wanted)
    })
}

/// Live processes of `uid` that carry the study path, never including `except_pid`.
pub fn domain_members(
    proc_root: &Path,
    uid: u32,
    study_compat_data: &Path,
    except_pid: u32,
) -> Vec<OwnedProcess> {
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| *pid != except_pid && *pid > 1)
        .filter(|pid| {
            std::fs::metadata(proc_root.join(pid.to_string())).is_ok_and(|meta| meta.uid() == uid)
        })
        .filter(|pid| carries_study_path(proc_root, *pid, study_compat_data))
        .filter_map(|pid| {
            Some(OwnedProcess {
                pid: Pid::new(pid),
                start_ticks: start_ticks(proc_root, pid)?,
            })
        })
        .collect()
}

/// Terminate every member of the domain, then verify none is left. Each signal goes through a
/// pidfd after re-reading the identity.
pub fn terminate_domain(
    proc_root: &Path,
    uid: u32,
    study_compat_data: &Path,
    except_pid: u32,
    grace: Duration,
) -> anyhow::Result<()> {
    for _ in 0..3 {
        let members = domain_members(proc_root, uid, study_compat_data, except_pid);
        if members.is_empty() {
            return Ok(());
        }
        for member in &members {
            terminate_exact(proc_root, member, grace)?;
        }
    }
    let left = domain_members(proc_root, uid, study_compat_data, except_pid);
    anyhow::ensure!(
        left.is_empty(),
        "{} study-prefix processes remain after cleanup",
        left.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::{Child, Command};

    use super::*;

    fn spawn_with(path: &str) -> Child {
        Command::new("sleep")
            .arg("60")
            .env("STEAM_COMPAT_DATA_PATH", path)
            .spawn()
            .unwrap()
    }

    #[test]
    fn only_processes_carrying_the_study_path_are_swept() {
        let uid = crate::syscall::geteuid();
        let study = Path::new("/nonexistent/stutter-test-study/compatdata");
        let mut mine = spawn_with("/nonexistent/stutter-test-study/compatdata");
        let mut other = spawn_with("/nonexistent/stutter-test-other/compatdata");
        let mut plain = Command::new("sleep").arg("60").spawn().unwrap();
        let members = domain_members(Path::new("/proc"), uid, study, 0);
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].pid.as_u32(), mine.id());
        terminate_domain(Path::new("/proc"), uid, study, 0, Duration::from_secs(2)).unwrap();
        assert!(mine.wait().is_ok());
        assert!(other.try_wait().unwrap().is_none());
        assert!(plain.try_wait().unwrap().is_none());
        let _ = other.kill();
        let _ = plain.kill();
        let _ = other.wait();
        let _ = plain.wait();
    }

    #[test]
    fn the_exempt_pid_is_never_claimed() {
        let uid = crate::syscall::geteuid();
        let study = Path::new("/nonexistent/stutter-test-exempt/compatdata");
        let mut child = spawn_with("/nonexistent/stutter-test-exempt/compatdata");
        assert!(domain_members(Path::new("/proc"), uid, study, child.id()).is_empty());
        let _ = child.kill();
        let _ = child.wait();
    }
}
