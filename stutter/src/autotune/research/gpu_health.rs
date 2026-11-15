//! GPU reset evidence for the research launch guard. A game can hang the GPU ring; the kernel then
//! resets the whole device, which on this desktop kills the compositor and the user's session. The
//! guard cannot prevent the hang, but it refuses to start a game soon after one (the device is in a
//! fragile state) and aborts a running attempt that coincides with one.
//!
//! Kernel messages are not readable directly (`dmesg_restrict`), and widening root access is not an
//! option, so the controller asks the journal, read-only, through an exact `journalctl` path that
//! the host configuration provides. An unreadable journal is *unknown*, never "no reset".

use std::{
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Kernel messages that mean the GPU hung or was reset (amdgpu).
const RESET_PATTERN: &str = "amdgpu.*(GPU reset begin|ring [a-z_0-9.]+ timeout|MODE1 reset)";
const QUERY_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_OUTPUT: u64 = 64 * 1024;

/// How many GPU hang/reset events were logged since `since_unix`.
pub trait GpuResetProbe: Send {
    fn resets_since(&mut self, since_unix: u64) -> anyhow::Result<u32>;
}

pub struct JournalGpuResetProbe {
    journalctl: PathBuf,
}

impl JournalGpuResetProbe {
    /// `journalctl` must be an absolute path (the host configuration supplies the store path).
    pub fn new(journalctl: PathBuf) -> anyhow::Result<Self> {
        anyhow::ensure!(
            journalctl.is_absolute() && journalctl.is_file(),
            "journalctl is not an absolute existing file"
        );
        Ok(Self { journalctl })
    }
}

impl GpuResetProbe for JournalGpuResetProbe {
    fn resets_since(&mut self, since_unix: u64) -> anyhow::Result<u32> {
        let mut child = Command::new(&self.journalctl)
            .args(["-k", "--no-pager", "-o", "cat", "--since"])
            .arg(format!("@{since_unix}"))
            .args(["-g", RESET_PATTERN])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no journal output pipe"))?;
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = (&mut stdout).take(MAX_OUTPUT).read_to_end(&mut bytes);
            bytes
        });
        let deadline = Instant::now() + QUERY_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("the kernel journal query timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let bytes = reader
            .join()
            .map_err(|_| anyhow::anyhow!("journal reader failed"))?;
        // `-g` exits 1 when nothing matched; anything else is an unreadable journal.
        anyhow::ensure!(
            status.success() || status.code() == Some(1),
            "the kernel journal could not be read"
        );
        Ok(bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .count()
            .min(u32::MAX as usize) as u32)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn script(body: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journalctl");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path)
    }

    #[test]
    fn matching_kernel_lines_are_counted() {
        let (_dir, path) = script("echo 'amdgpu: GPU reset begin!'; echo 'amdgpu: MODE1 reset'");
        assert_eq!(
            JournalGpuResetProbe::new(path)
                .unwrap()
                .resets_since(0)
                .unwrap(),
            2
        );
    }

    #[test]
    fn no_match_is_zero_and_an_unreadable_journal_is_unknown() {
        let (_dir, none) = script("exit 1");
        assert_eq!(
            JournalGpuResetProbe::new(none)
                .unwrap()
                .resets_since(0)
                .unwrap(),
            0
        );
        let (_dir2, broken) = script("exit 2");
        assert!(
            JournalGpuResetProbe::new(broken)
                .unwrap()
                .resets_since(0)
                .is_err()
        );
    }

    #[test]
    fn a_hung_query_times_out_as_unknown_and_a_relative_path_is_refused() {
        let (_dir, hung) = script("sleep 30");
        let started = Instant::now();
        assert!(
            JournalGpuResetProbe::new(hung)
                .unwrap()
                .resets_since(0)
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(8));
        assert!(JournalGpuResetProbe::new(PathBuf::from("journalctl")).is_err());
    }
}
