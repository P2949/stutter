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
    path::{Path, PathBuf},
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
            // `-k` alone means the current boot only; a reset in an earlier boot must stay visible.
            .args(["-k", "-b", "all", "--no-pager", "-o", "cat", "--since"])
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

/// File (in the private study directory) written when a GPU hang or reset is detected. It is a
/// durable campaign-independent health stop: it survives controller restarts and reboots, and only
/// a deliberate human acknowledgement newer than it clears the stop; time never does.
pub const BLOCKED_FILE: &str = "gpu-stability-blocked.json";
/// Written by a human (`date +%s > gpu-stability-ack`) after reviewing the GPU events. Only events
/// after this time count, and a block recorded before it no longer applies.
pub const ACK_FILE: &str = "gpu-stability-ack";
/// Consecutive unreadable journal checks during an attempt that are treated as an unknown, fatal
/// health state.
const MAX_UNKNOWN_POLLS: u32 = 3;
/// A running attempt is checked for a GPU reset at most this often.
const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// The window check runs every second; the journal is not queried that often.
const ADMISSION_CACHE: Duration = Duration::from_secs(10);

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The port-side state of the guard: one probe, the durable health state and the running attempt.
pub struct GpuGuard {
    probe: Box<dyn GpuResetProbe>,
    /// The private directory holding the durable block marker and the human acknowledgement.
    state_dir: PathBuf,
    pub(super) attempt_started_unix: Option<u64>,
    pub(super) last_poll: Option<Instant>,
    reset_seen: bool,
    unknown_polls: u32,
    pub(super) admission: Option<(Instant, bool)>,
}

fn read_unix(path: &Path) -> Option<u64> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(64)
        .read_to_string(&mut text)
        .ok()?;
    text.trim().parse().ok()
}

impl GpuGuard {
    pub fn new(probe: Box<dyn GpuResetProbe>, state_dir: PathBuf) -> Self {
        Self {
            probe,
            state_dir,
            attempt_started_unix: None,
            last_poll: None,
            reset_seen: false,
            unknown_polls: 0,
            admission: None,
        }
    }

    /// The human acknowledgement time. Missing or malformed means "never acknowledged", so every
    /// recorded GPU event counts.
    fn acknowledged_unix(&self) -> u64 {
        read_unix(&self.state_dir.join(ACK_FILE)).unwrap_or(0)
    }

    /// Whether a durable block, newer than any acknowledgement, is on record.
    pub fn blocked(&self) -> bool {
        let marker = self.state_dir.join(BLOCKED_FILE);
        if !marker.exists() {
            return false;
        }
        // A marker whose time cannot be read is a block: ownership of the decision stays human.
        let detected = std::fs::read(&marker)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value["detected_unix"].as_u64())
            .unwrap_or(u64::MAX);
        detected > self.acknowledged_unix()
    }

    /// Record the stop durably (private file, never overwritten by a later event).
    fn record_block(&self, source: &str, events: u32) {
        use std::os::unix::fs::OpenOptionsExt;
        let body = serde_json::json!({
            "detected_unix": unix_now(),
            "events": events,
            "source": source,
        });
        let result = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.state_dir.join(BLOCKED_FILE))
            .and_then(|mut file| {
                use std::io::Write;
                file.write_all(body.to_string().as_bytes())?;
                file.sync_all()
            });
        match result {
            Ok(()) => {}
            // An earlier block stays; a newer acknowledgement is what supersedes it.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => log::error!("the GPU stability block could not be recorded: {error}"),
        }
    }

    /// A launch request was just recorded: watch for resets from now on.
    pub fn begin_attempt(&mut self) {
        self.attempt_started_unix = Some(unix_now());
        self.reset_seen = false;
        self.unknown_polls = 0;
        self.last_poll = None;
    }

    /// Admission. No time cooldown: a durable block, any GPU event since the last human
    /// acknowledgement (across every boot) or an unreadable journal all withhold the launch.
    pub fn quiet_for_admission(&mut self) -> bool {
        if self.blocked() {
            return false;
        }
        if let Some((at, quiet)) = self.admission
            && at.elapsed() < ADMISSION_CACHE
        {
            return quiet;
        }
        let quiet = match self.probe.resets_since(self.acknowledged_unix()) {
            Ok(0) => true,
            Ok(events) => {
                self.record_block("admission", events);
                false
            }
            Err(error) => {
                log::warn!(
                    "GPU health is unknown (journal unreadable); admission withheld: {error:#}"
                );
                false
            }
        };
        self.admission = Some((Instant::now(), quiet));
        quiet
    }

    /// During an attempt: a reset since the launch request, or repeated inability to check, is
    /// fatal and sticky. It also writes the durable block.
    pub fn reset_during_attempt(&mut self) -> bool {
        if self.reset_seen {
            return true;
        }
        let Some(started) = self.attempt_started_unix else {
            return false;
        };
        if self
            .last_poll
            .is_some_and(|at| at.elapsed() < POLL_INTERVAL)
        {
            return false;
        }
        self.last_poll = Some(Instant::now());
        match self.probe.resets_since(started.saturating_sub(2)) {
            Ok(0) => self.unknown_polls = 0,
            Ok(events) => {
                self.record_block("attempt", events);
                self.reset_seen = true;
            }
            Err(error) => {
                self.unknown_polls += 1;
                log::warn!("GPU health check failed during an attempt: {error:#}");
                if self.unknown_polls >= MAX_UNKNOWN_POLLS {
                    self.record_block("unknown_during_attempt", 0);
                    self.reset_seen = true;
                }
            }
        }
        self.reset_seen
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
