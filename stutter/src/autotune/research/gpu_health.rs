//! GPU reset evidence for the research launch guard. A game can hang the GPU ring; the kernel then
//! resets the whole device, which on this desktop kills the compositor and the user's session. The
//! guard cannot prevent the hang, but it refuses to start a game after one (the device is in a
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

/// What the kernel journal holds about GPU hangs/resets from some time on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResetScan {
    pub count: u32,
    /// The newest matching event's time, when the journal gave timestamps.
    pub latest_unix: Option<u64>,
}

impl ResetScan {
    pub const fn quiet() -> Self {
        Self {
            count: 0,
            latest_unix: None,
        }
    }
}

/// GPU hang/reset events logged since `since_unix`.
pub trait GpuResetProbe: Send {
    fn resets_since(&mut self, since_unix: u64) -> anyhow::Result<ResetScan>;
}

pub struct JournalGpuResetProbe {
    journalctl: PathBuf,
}

impl JournalGpuResetProbe {
    /// Production constructor: the host configuration supplies the exact store path, which must be
    /// an existing regular file owned by root that no one else can write.
    pub fn trusted(journalctl: PathBuf) -> anyhow::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let canonical = std::fs::canonicalize(&journalctl)?;
        let meta = std::fs::metadata(&canonical)?;
        anyhow::ensure!(
            meta.is_file() && meta.uid() == 0 && meta.mode() & 0o022 == 0,
            "journalctl is not a root-owned, non-writable regular file"
        );
        Self::new(journalctl)
    }

    /// `journalctl` must be an absolute path.
    pub fn new(journalctl: PathBuf) -> anyhow::Result<Self> {
        anyhow::ensure!(
            journalctl.is_absolute() && journalctl.is_file(),
            "journalctl is not an absolute existing file"
        );
        Ok(Self { journalctl })
    }
}

impl GpuResetProbe for JournalGpuResetProbe {
    fn resets_since(&mut self, since_unix: u64) -> anyhow::Result<ResetScan> {
        let mut child = Command::new(&self.journalctl)
            // `-k` alone means the current boot only; a reset in an earlier boot must stay visible.
            .args([
                "-k",
                "-b",
                "all",
                "--no-pager",
                "-o",
                "short-unix",
                "--since",
            ])
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
        let mut count = 0usize;
        let mut latest: Option<u64> = None;
        for line in bytes.split(|byte| *byte == b'\n').filter(|l| !l.is_empty()) {
            count += 1;
            let stamp = line
                .split(|byte| *byte == b' ')
                .next()
                .and_then(|token| std::str::from_utf8(token).ok())
                .and_then(|token| token.split('.').next())
                .and_then(|seconds| seconds.parse::<u64>().ok());
            if let Some(seconds) = stamp {
                latest = Some(latest.map_or(seconds, |old| old.max(seconds)));
            }
        }
        Ok(ResetScan {
            count: count.min(u32::MAX as usize) as u32,
            latest_unix: latest,
        })
    }
}

/// The original single-marker file name; still honoured as an incident record.
pub const LEGACY_BLOCKED_FILE: &str = "gpu-stability-blocked.json";
/// Every detected hang/reset is its own immutable, fsynced file with this prefix, so a later reset
/// can never be hidden by an earlier record or acknowledgement.
pub const INCIDENT_PREFIX: &str = "gpu-stability-incident-";
/// One line per reviewed incident, written by the human: `<incident file name without .json> <first
/// 16 hex of the incident file's sha256>`. It binds the acknowledgement to that exact record; a
/// timestamp, a different incident or an edited record never matches.
pub const ACK_FILE: &str = "gpu-stability-acks";
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
    /// The private directory holding the incident records and the human acknowledgements.
    state_dir: PathBuf,
    pub(super) attempt_started_unix: Option<u64>,
    pub(super) last_poll: Option<Instant>,
    reset_seen: bool,
    /// An incident could not be written durably: blocked for the life of this process.
    latched: bool,
    unknown_polls: u32,
    pub(super) admission: Option<(Instant, bool)>,
}

struct Incident {
    stem: String,
    hash16: String,
    /// Upper bound of the events this record covers; `None` when the record is unreadable.
    covers_until: Option<u64>,
}

fn incident_name(name: &str) -> bool {
    name.ends_with(".json") && (name == LEGACY_BLOCKED_FILE || name.starts_with(INCIDENT_PREFIX))
}

fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(limit)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

impl GpuGuard {
    pub fn new(probe: Box<dyn GpuResetProbe>, state_dir: PathBuf) -> Self {
        Self {
            probe,
            state_dir,
            attempt_started_unix: None,
            last_poll: None,
            reset_seen: false,
            latched: false,
            unknown_polls: 0,
            admission: None,
        }
    }

    /// Every incident on record; `None` when the directory cannot be listed (which blocks).
    fn incidents(&self) -> Option<Vec<Incident>> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&self.state_dir).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !incident_name(&name) {
                continue;
            }
            let bytes = read_bounded(&entry.path(), 64 * 1024);
            let covers_until = bytes.as_deref().and_then(|raw| {
                let value: serde_json::Value = serde_json::from_slice(raw).ok()?;
                let detected = value["detected_unix"].as_u64()?;
                Some(
                    value["last_event_unix"]
                        .as_u64()
                        .map_or(detected, |event| event.min(detected)),
                )
            });
            found.push(Incident {
                stem: name.trim_end_matches(".json").to_string(),
                hash16: bytes
                    .as_deref()
                    .map(|raw| stutter_core::hash::sha256_hex(raw)[..16].to_string())
                    .unwrap_or_default(),
                covers_until,
            });
        }
        Some(found)
    }

    /// The acknowledgement lines, only from a regular file owned by this user that nobody else can
    /// write. Anything else acknowledges nothing.
    fn acknowledgements(&self) -> Vec<(String, String)> {
        use std::os::unix::fs::MetadataExt;
        let path = self.state_dir.join(ACK_FILE);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            return Vec::new();
        };
        if !meta.is_file() || meta.mode() & 0o022 != 0 || meta.uid() != crate::syscall::geteuid() {
            return Vec::new();
        }
        let Some(bytes) = read_bounded(&path, 64 * 1024) else {
            return Vec::new();
        };
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let (id, hash) = (parts.next()?, parts.next()?);
                parts
                    .next()
                    .is_none()
                    .then(|| (id.to_string(), hash.to_string()))
            })
            .collect()
    }

    fn acknowledged(acks: &[(String, String)], incident: &Incident) -> bool {
        !incident.hash16.is_empty()
            && acks
                .iter()
                .any(|(id, hash)| *id == incident.stem && *hash == incident.hash16)
    }

    /// Whether any incident lacks its own acknowledgement (or the state cannot be read at all).
    pub fn blocked(&self) -> bool {
        if self.latched {
            return true;
        }
        let Some(incidents) = self.incidents() else {
            return true;
        };
        if incidents.is_empty() {
            return false;
        }
        let acks = self.acknowledgements();
        incidents
            .iter()
            .any(|incident| !Self::acknowledged(&acks, incident))
    }

    /// Journal lines at or before this time belong to acknowledged incidents. Unreadable records
    /// contribute nothing, so their events are found (and blocked) again.
    fn reviewed_until(&self) -> u64 {
        let acks = self.acknowledgements();
        self.incidents()
            .unwrap_or_default()
            .iter()
            .filter(|incident| Self::acknowledged(&acks, incident))
            .filter_map(|incident| incident.covers_until)
            .max()
            .map_or(0, |until| until.saturating_add(1))
    }

    /// Persist a new incident (private, unique, fsynced with its directory). A failure latches the
    /// block for this process; it never silently drops the incident.
    fn record_block(&mut self, source: &str, scan: ResetScan) {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let detected = unix_now();
        let body = serde_json::json!({
            "version": 1,
            "detected_unix": detected,
            "last_event_unix": scan.latest_unix,
            "events": scan.count,
            "source": source,
            "acknowledge": "review the kernel journal, then append '<this file name without .json> <first 16 hex of sha256sum of this file>' to gpu-stability-acks (mode 0600)",
        })
        .to_string();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let mut outcome = Err(std::io::Error::other("no name"));
        for attempt in 0..8u32 {
            let name = format!(
                "{INCIDENT_PREFIX}{detected}-{nanos}-{}-{attempt}.json",
                std::process::id()
            );
            outcome = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(self.state_dir.join(name))
                .and_then(|mut file| {
                    file.write_all(body.as_bytes())?;
                    file.sync_all()
                });
            match &outcome {
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                _ => break,
            }
        }
        let outcome = outcome.and_then(|()| std::fs::File::open(&self.state_dir)?.sync_all());
        if let Err(error) = outcome {
            log::error!("the GPU stability incident could not be recorded: {error}");
            self.latched = true;
        }
    }

    /// A launch request was just recorded: watch for resets from now on.
    pub fn begin_attempt(&mut self) {
        self.attempt_started_unix = Some(unix_now());
        self.reset_seen = false;
        self.unknown_polls = 0;
        self.last_poll = None;
    }

    /// Admission (cached for a few seconds). No time cooldown: an unacknowledged incident, any GPU
    /// event after the reviewed ones (across every boot) or an unreadable journal withhold launch.
    pub fn quiet_for_admission(&mut self) -> bool {
        if self.blocked() {
            return false;
        }
        if let Some((at, quiet)) = self.admission
            && at.elapsed() < ADMISSION_CACHE
        {
            return quiet;
        }
        self.admit_now()
    }

    /// The same decision without the cache: used at the last moment before a launch is sent.
    pub fn admit_now(&mut self) -> bool {
        if self.blocked() {
            return false;
        }
        let quiet = match self.probe.resets_since(self.reviewed_until()) {
            Ok(scan) if scan.count == 0 => true,
            Ok(scan) => {
                self.record_block("admission", scan);
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
    /// fatal and sticky. It also writes the durable incident.
    pub fn reset_during_attempt(&mut self) -> bool {
        if self.reset_seen || self.latched {
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
            Ok(scan) if scan.count == 0 => self.unknown_polls = 0,
            Ok(scan) => {
                self.record_block("attempt", scan);
                self.reset_seen = true;
            }
            Err(error) => {
                self.unknown_polls += 1;
                log::warn!("GPU health check failed during an attempt: {error:#}");
                if self.unknown_polls >= MAX_UNKNOWN_POLLS {
                    self.record_block("unknown_during_attempt", ResetScan::quiet());
                    self.reset_seen = true;
                }
            }
        }
        self.reset_seen
    }
}

#[cfg(test)]
#[path = "gpu_health_tests.rs"]
mod tests;
