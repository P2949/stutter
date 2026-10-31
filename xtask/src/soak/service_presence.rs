//! Verified live dwell. Downtime, suspend gaps, replaced processes and stale samples count zero.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::records::unix_ms;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Identity {
    pid: u32,
    ticks: u64,
    boot: String,
    attestation: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Interval {
    identity: Identity,
    start_unix_ms: u64,
    end_unix_ms: u64,
    elapsed_ms: u64,
}

#[derive(Debug)]
pub(super) struct Presence {
    dir: PathBuf,
    target: Arc<Mutex<Option<Identity>>>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<anyhow::Result<()>>>>,
    _lock: fs::File,
}

impl Presence {
    pub(super) fn start(dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(dir.join("presence-attestations"))?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("presence.lock"))?;
        lock.try_lock()
            .context("another service dwell collector already owns this soak")?;
        let target = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (sample_target, stopped, output) = (target.clone(), stop.clone(), dir.to_owned());
        let thread = std::thread::spawn(move || collect(&output, &sample_target, &stopped));
        Ok(Self {
            dir: dir.to_owned(),
            target,
            stop,
            thread: Mutex::new(Some(thread)),
            _lock: lock,
        })
    }

    pub(super) fn observe(&self, report: &Value) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(report)?;
        let digest = stutter_core::hash::sha256_hex(&bytes);
        let path = self
            .dir
            .join("presence-attestations")
            .join(format!("{digest}.json"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(&bytes)?;
                file.sync_all()?;
                fs::File::open(path.parent().context("attestation parent missing")?)?.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => ensure!(
                fs::read(path)? == bytes,
                "immutable runtime attestation differs"
            ),
            Err(error) => return Err(error.into()),
        }
        let id = Identity {
            pid: u32::try_from(
                report["controller_process"]["pid"]
                    .as_u64()
                    .context("missing controller PID")?,
            )?,
            ticks: report["controller_process"]["starttime_ticks"]
                .as_u64()
                .context("missing controller start ticks")?,
            boot: report["boot_id"]
                .as_str()
                .context("missing actual boot")?
                .into(),
            attestation: digest,
        };
        *self
            .target
            .lock()
            .map_err(|_| anyhow::anyhow!("presence state poisoned"))? = Some(id);
        Ok(())
    }

    pub(super) fn finish(&self) -> anyhow::Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self
            .thread
            .lock()
            .map_err(|_| anyhow::anyhow!("presence join state poisoned"))?
            .take()
        {
            let result = thread
                .join()
                .map_err(|_| anyhow::anyhow!("presence collector panicked"))?;
            if let Err(error) = &result {
                fs::write(self.dir.join("presence-error.txt"), format!("{error:#}"))?;
            }
            result?;
        }
        Ok(())
    }
}
impl Drop for Presence {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn alive(id: &Identity) -> bool {
    if stutter_core::boot::current_boot_id().ok().as_ref() != Some(&id.boot) {
        return false;
    }
    let Ok(stat) = fs::read_to_string(format!("/proc/{}/stat", id.pid)) else {
        return false;
    };
    let Some((_, fields)) = stat.rsplit_once(") ") else {
        return false;
    };
    matches!(fields.chars().next(), Some('R' | 'S' | 'D' | 'I'))
        && fields
            .split_whitespace()
            .nth(19)
            .and_then(|ticks| ticks.parse::<u64>().ok())
            == Some(id.ticks)
}

fn usable(previous: &Identity, current: &Identity, wall_ms: u64, monotonic_ms: u64) -> bool {
    previous == current && (1..=2000).contains(&wall_ms) && (1..=2000).contains(&monotonic_ms)
}

fn collect(dir: &Path, target: &Mutex<Option<Identity>>, stop: &AtomicBool) -> anyhow::Result<()> {
    let mut output = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("presence.jsonl"))?;
    let mut previous: Option<(Identity, u64, Instant)> = None;
    while !stop.load(Ordering::SeqCst) {
        let identity = target
            .lock()
            .map_err(|_| anyhow::anyhow!("presence target poisoned"))?
            .clone();
        let now = unix_ms();
        let monotonic = Instant::now();
        match identity.filter(alive) {
            Some(id) => {
                if let Some((old, before, clock)) = previous.take() {
                    let wall = now.saturating_sub(before);
                    let elapsed = u64::try_from(monotonic.duration_since(clock).as_millis())?;
                    if usable(&old, &id, wall, elapsed) {
                        let mut bytes = serde_json::to_vec(&Interval {
                            identity: id.clone(),
                            start_unix_ms: before,
                            end_unix_ms: now,
                            elapsed_ms: wall.min(elapsed),
                        })?;
                        bytes.push(b'\n');
                        output.write_all(&bytes)?;
                        output.sync_data()?;
                    }
                }
                previous = Some((id, now, monotonic));
            }
            None => previous = None,
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

pub(super) fn verified_ms(
    dir: &Path,
    state: &crate::service_campaign::model::State,
) -> anyhow::Result<u64> {
    use std::io::BufRead;
    ensure!(
        !dir.join("presence-error.txt").exists(),
        "presence collector failed"
    );
    let path = dir.join("presence.jsonl");
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0_u64;
    let mut end = 0_u64;
    let mut checked = std::collections::BTreeMap::new();
    let mut input = std::io::BufReader::new(fs::File::open(path)?);
    let mut line = Vec::new();
    while input.read_until(b'\n', &mut line)? > 0 {
        // An in-flight append is not evidence; only complete durable intervals count.
        if line.last() != Some(&b'\n') {
            break;
        }
        let row: Interval = serde_json::from_slice(&line)?;
        line.clear();
        ensure!(
            row.start_unix_ms >= end
                && row.end_unix_ms > row.start_unix_ms
                && (1..=2000).contains(&row.elapsed_ms)
                && row.end_unix_ms - row.start_unix_ms <= 2000
                && row.elapsed_ms <= row.end_unix_ms - row.start_unix_ms,
            "invalid, overlapping or stale presence interval"
        );
        ensure!(
            row.identity.attestation.len() == 64
                && row
                    .identity
                    .attestation
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "invalid presence attestation identity"
        );
        if let Some(identity) = checked.get(&row.identity.attestation) {
            ensure!(
                identity == &row.identity,
                "presence identity changed under an existing attestation"
            );
        } else {
            let bytes = fs::read(
                dir.join("presence-attestations")
                    .join(format!("{}.json", row.identity.attestation)),
            )?;
            ensure!(
                stutter_core::hash::sha256_hex(&bytes) == row.identity.attestation,
                "runtime attestation changed"
            );
            let report: Value = serde_json::from_slice(&bytes)?;
            super::service::attest(&report, state)?;
            ensure!(
                report["controller_process"]["pid"] == row.identity.pid
                    && report["controller_process"]["starttime_ticks"] == row.identity.ticks
                    && report["boot_id"] == row.identity.boot,
                "presence interval disagrees with its root runtime attestation"
            );
            checked.insert(row.identity.attestation.clone(), row.identity.clone());
        }
        total = total
            .checked_add(row.elapsed_ms)
            .context("presence duration overflow")?;
        end = row.end_unix_ms;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_gap_reboot_replacement_and_paused_wall_clock_never_count() {
        let id = Identity {
            pid: 42,
            ticks: 100,
            boot: "boot-one".into(),
            attestation: "a".repeat(64),
        };
        assert!(usable(&id, &id, 1000, 1000));
        assert!(!usable(&id, &id, 30_000, 1000));
        assert!(!usable(&id, &id, 1000, 30_000));
        assert!(!usable(&id, &id, 0, 1000));
        let mut replacement = id.clone();
        replacement.ticks += 1;
        assert!(!usable(&id, &replacement, 1000, 1000));
        replacement = id.clone();
        replacement.boot = "boot-two".into();
        assert!(!usable(&id, &replacement, 1000, 1000));
    }
}
