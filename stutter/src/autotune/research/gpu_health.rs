//! GPU reset evidence for the research launch guard. A game can hang the GPU ring; the kernel then
//! resets the whole device, which on this desktop kills the compositor and the user's session. The
//! guard cannot prevent the hang, but it refuses to start a game after one (the device is in a
//! fragile state) and aborts a running attempt that coincides with one.
//!
//! Kernel messages are not readable directly (`dmesg_restrict`), and widening root access is not an
//! option, so the controller asks the journal, read-only, through an exact `journalctl` path that
//! the host configuration provides. An unreadable journal is *unknown*, never "no reset", and an
//! empty search result is only believed after the journal is independently shown to be readable.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

/// Kernel messages that mean the GPU hung or was reset (amdgpu).
const RESET_PATTERN: &str = "amdgpu.*(GPU reset begin|ring [a-z_0-9.]+ timeout|MODE1 reset)";
const QUERY_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_OUTPUT: u64 = 64 * 1024;
const MAX_STDERR: u64 = 8 * 1024;
const MAX_LINE_HASHES: usize = 256;

/// What the kernel journal holds about GPU hangs/resets from some time on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResetScan {
    pub count: u32,
    /// The newest matching event's time, when the journal gave timestamps.
    pub latest_unix: Option<u64>,
    /// SHA-256 of each matching journal line (microsecond timestamps make them unique), so an
    /// event that was already reviewed is recognised exactly and never lost to a time boundary.
    pub line_hashes: Vec<String>,
}

impl ResetScan {
    pub fn quiet() -> Self {
        Self::default()
    }

    /// A scan with `count` events and no per-line identity (tests only).
    #[cfg(test)]
    pub fn events(count: u32, latest_unix: Option<u64>) -> Self {
        Self {
            count,
            latest_unix,
            line_hashes: Vec::new(),
        }
    }
}

/// GPU hang/reset events logged since `since_unix`, inclusive.
pub trait GpuResetProbe: Send {
    fn resets_since(&mut self, since_unix: u64) -> anyhow::Result<ResetScan>;
}

pub struct JournalGpuResetProbe {
    journalctl: PathBuf,
}

struct Output {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
}

fn stamp_of(line: &[u8]) -> Option<u64> {
    let token = line.split(|byte| *byte == b' ').next()?;
    let text = std::str::from_utf8(token).ok()?;
    let (seconds, micros) = text.split_once('.')?;
    (!micros.is_empty() && micros.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| seconds.parse().ok())
        .flatten()
}

impl JournalGpuResetProbe {
    /// Production constructor: the host configuration supplies the exact store path, which must be
    /// an existing regular file owned by root that no one else can write. The vetted, resolved
    /// target is the one that is executed, never the original path.
    pub fn trusted(journalctl: PathBuf) -> anyhow::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let canonical = std::fs::canonicalize(&journalctl)?;
        let meta = std::fs::metadata(&canonical)?;
        anyhow::ensure!(
            meta.is_file() && meta.uid() == 0 && meta.mode() & 0o022 == 0,
            "journalctl is not a root-owned, non-writable regular file"
        );
        Self::new(canonical)
    }

    /// `journalctl` must be an absolute path.
    pub fn new(journalctl: PathBuf) -> anyhow::Result<Self> {
        anyhow::ensure!(
            journalctl.is_absolute() && journalctl.is_file(),
            "journalctl is not an absolute existing file"
        );
        Ok(Self { journalctl })
    }

    fn run(&self, args: &[&str]) -> anyhow::Result<Output> {
        let mut child = Command::new(&self.journalctl)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no journal output pipe"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("no journal error pipe"))?;
        let out_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = (&mut stdout).take(MAX_OUTPUT + 1).read_to_end(&mut bytes);
            bytes
        });
        let err_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = (&mut stderr).take(MAX_STDERR).read_to_end(&mut bytes);
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
        let mut stdout = out_reader
            .join()
            .map_err(|_| anyhow::anyhow!("journal reader failed"))?;
        let stderr = err_reader
            .join()
            .map_err(|_| anyhow::anyhow!("journal error reader failed"))?;
        let truncated = stdout.len() as u64 > MAX_OUTPUT;
        stdout.truncate(MAX_OUTPUT as usize);
        Ok(Output {
            status,
            stdout,
            stderr,
            truncated,
        })
    }

    /// The journal is independently shown to be readable: the current boot's kernel log yields a
    /// timestamped line and the boot list is non-empty. Without this an empty search proves nothing
    /// (`journalctl -g` exits 1 both for "no match" and for "no journal files").
    fn qualify(&self) -> anyhow::Result<()> {
        let kernel = self.run(&["-k", "-b", "0", "--no-pager", "-o", "short-unix", "-n", "1"])?;
        anyhow::ensure!(
            kernel.status.success()
                && kernel
                    .stdout
                    .split(|byte| *byte == b'\n')
                    .find(|line| !line.is_empty())
                    .and_then(stamp_of)
                    .is_some(),
            "the kernel journal of this boot is not readable"
        );
        let boots = self.run(&["--list-boots", "--no-pager"])?;
        anyhow::ensure!(
            boots.status.success() && boots.stdout.iter().any(|byte| !byte.is_ascii_whitespace()),
            "the journal boot list is not readable"
        );
        Ok(())
    }
}

impl GpuResetProbe for JournalGpuResetProbe {
    fn resets_since(&mut self, since_unix: u64) -> anyhow::Result<ResetScan> {
        self.qualify()?;
        let since = format!("@{since_unix}");
        // `-k` alone means the current boot only; a reset in an earlier boot must stay visible.
        let found = self.run(&[
            "-k",
            "-b",
            "all",
            "--no-pager",
            "-o",
            "short-unix",
            "--since",
            &since,
            "-g",
            RESET_PATTERN,
        ])?;
        let note = |bytes: &[u8]| {
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim();
            text.is_empty() || text == "-- No entries --"
        };
        if found.status.code() == Some(1) {
            // Exit 1 is ambiguous. It is an empty result only with nothing on stderr, nothing
            // unexpected on stdout, and a journal that is still readable afterwards.
            anyhow::ensure!(
                note(&found.stderr) && note(&found.stdout),
                "the journal search failed rather than found nothing"
            );
            self.qualify()?;
            return Ok(ResetScan::quiet());
        }
        anyhow::ensure!(
            found.status.success(),
            "the kernel journal could not be read"
        );
        let mut scan = ResetScan::quiet();
        let lines: Vec<&[u8]> = found
            .stdout
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .collect();
        for (index, line) in lines.iter().enumerate() {
            // A cut-off final line of a truncated capture is not malformed; it is still an event.
            let partial = found.truncated && index + 1 == lines.len();
            let stamp = stamp_of(line);
            anyhow::ensure!(
                partial || (stamp.is_some() && String::from_utf8_lossy(line).contains("amdgpu")),
                "the journal output is not a list of amdgpu events"
            );
            scan.count = scan.count.saturating_add(1);
            scan.latest_unix = match (scan.latest_unix, stamp) {
                (Some(old), Some(new)) => Some(old.max(new)),
                (old, new) => old.or(new),
            };
            if scan.line_hashes.len() < MAX_LINE_HASHES {
                scan.line_hashes.push(stutter_core::hash::sha256_hex(line));
            }
        }
        // A successful search that printed nothing is not a result either.
        anyhow::ensure!(
            scan.count > 0,
            "the journal search succeeded without output"
        );
        // A journal that vanished mid-query could have cut the output short.
        self.qualify()?;
        Ok(scan)
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
    /// SHA-256 of each journal line this record covers.
    lines: Vec<String>,
}

fn incident_name(name: &str) -> bool {
    name.ends_with(".json") && (name == LEGACY_BLOCKED_FILE || name.starts_with(INCIDENT_PREFIX))
}

/// Read a record the guard trusts: opened without following a symlink, a regular file owned by
/// this user that nobody else can write, and no larger than `limit`. Anything else is not read.
fn read_trusted(path: &Path, limit: u64) -> Option<Vec<u8>> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file()
        || meta.uid() != crate::syscall::geteuid()
        || meta.mode() & 0o022 != 0
        || meta.len() > limit
    {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 == meta.len()).then_some(bytes)
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
            let bytes = read_trusted(&entry.path(), 64 * 1024);
            let covers_until = bytes.as_deref().and_then(|raw| {
                let value: serde_json::Value = serde_json::from_slice(raw).ok()?;
                let detected = value["detected_unix"].as_u64()?;
                Some(
                    value["last_event_unix"]
                        .as_u64()
                        .map_or(detected, |event| event.min(detected)),
                )
            });
            let lines = bytes
                .as_deref()
                .and_then(|raw| serde_json::from_slice::<serde_json::Value>(raw).ok())
                .and_then(|value| {
                    value["line_sha256"].as_array().map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect::<Vec<_>>()
                    })
                })
                .unwrap_or_default();
            found.push(Incident {
                stem: name.trim_end_matches(".json").to_string(),
                hash16: bytes
                    .as_deref()
                    .map(|raw| stutter_core::hash::sha256_hex(raw)[..16].to_string())
                    .unwrap_or_default(),
                covers_until,
                lines,
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
        let Some(bytes) = read_trusted(&path, 64 * 1024) else {
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

    /// What acknowledged incidents already cover: events at or before the returned time, and the
    /// exact journal lines they recorded (so an event in the same second as a reviewed one is not
    /// lost, only the reviewed line itself is skipped). Unreadable records cover nothing, so their
    /// events are found (and blocked) again.
    fn reviewed(&self) -> (u64, std::collections::BTreeSet<String>) {
        let acks = self.acknowledgements();
        let incidents = self.incidents().unwrap_or_default();
        let reviewed: Vec<&Incident> = incidents
            .iter()
            .filter(|incident| Self::acknowledged(&acks, incident))
            .collect();
        let since = reviewed
            .iter()
            .filter_map(|incident| incident.covers_until)
            .max()
            .unwrap_or(0);
        let lines = reviewed
            .iter()
            .flat_map(|incident| incident.lines.iter().cloned())
            .collect();
        (since, lines)
    }

    /// Events in `scan` that no acknowledged incident covers.
    fn fresh_events(scan: &ResetScan, reviewed: &std::collections::BTreeSet<String>) -> u32 {
        let known: u32 = scan
            .line_hashes
            .iter()
            .filter(|hash| reviewed.contains(*hash))
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        scan.count.saturating_sub(known)
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
            "line_sha256": scan.line_hashes,
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
        let (since, reviewed) = self.reviewed();
        let quiet = match self.probe.resets_since(since) {
            Ok(scan) if Self::fresh_events(&scan, &reviewed) == 0 => true,
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
