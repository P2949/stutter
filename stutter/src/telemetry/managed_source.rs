//! One game-run launch's MangoHud log as a live frame source.
//!
//! The source owns a confined tail session over the launch's capture directory and publishes its
//! health to a shared [`FrameSourceStatus`] that the runtime evaluates against the target on every
//! tick. It is polled, never threaded: each poll reads at most the room it is given, so frames stay
//! in the file under backpressure instead of being dropped. If the launched process is gone or was
//! replaced, the source reports itself gone and trust ends at once.
use std::{
    fs,
    sync::{Arc, Mutex},
    time::Instant,
};

use super::{
    frame_source::{MeasurementStage, SourceClaim},
    launch_discovery::DiscoveredLaunch,
    live_source::{FrameSourceStatus, SharedFrameSource},
    proc_view::live_process,
    session::ConfinedDir,
};
use crate::mangohud::{MangoHudTailSession, TailConfig, TailState, TailedFrame};

/// The newest MangoHud frame log in `dir`: a `.csv` that is not a summary file.
pub fn newest_frame_log(dir: &std::path::Path) -> Option<String> {
    let mut logs: Vec<(std::time::SystemTime, String)> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let meta = fs::symlink_metadata(entry.path()).ok()?;
            if meta.file_type().is_file() && name.ends_with(".csv") && !name.contains("summary") {
                Some((meta.modified().ok()?, name))
            } else {
                None
            }
        })
        .collect();
    logs.sort();
    logs.pop().map(|(_, name)| name)
}

const DRAIN_ROOM: usize = 4096;
const MAX_DRAIN_ROUNDS: usize = 64;

pub struct PollOutcome {
    pub frames: Vec<TailedFrame>,
    /// The launched process no longer exists or is not the one recorded.
    pub gone: bool,
}

pub struct ManagedFrameSource {
    launch: DiscoveredLaunch,
    uid: u32,
    session: Option<MangoHudTailSession>,
    status: SharedFrameSource,
    config: TailConfig,
}

impl ManagedFrameSource {
    pub fn new(launch: DiscoveredLaunch, uid: u32) -> Self {
        let claim = SourceClaim {
            stage: MeasurementStage::ApplicationFrame,
            producer_pid: Some(launch.record.pid),
            producer_start_ticks: Some(launch.record.start_ticks),
            // Set once the log file has been opened and its owner checked on the descriptor.
            file_owner_uid: None,
            launch_id_matched: true,
        };
        Self {
            launch,
            uid,
            session: None,
            status: Arc::new(Mutex::new(FrameSourceStatus::new(claim))),
            config: TailConfig::default(),
        }
    }

    /// The shared status the runtime evaluates.
    pub fn shared_status(&self) -> SharedFrameSource {
        Arc::clone(&self.status)
    }

    fn publish(&self, update: impl FnOnce(&mut FrameSourceStatus)) {
        if let Ok(mut status) = self.status.lock() {
            update(&mut status);
        }
    }

    /// Reads and discards every frame already in the log, so a phase boundary does not let frames
    /// produced before it land in the next phase's window. Bounded: at most `MAX_DRAIN_ROUNDS`
    /// polls of `DRAIN_ROOM` frames. Returns how many frames were discarded.
    pub fn discard_pending(&mut self, proc_root: &std::path::Path, now: Instant) -> usize {
        let mut discarded = 0;
        for _ in 0..MAX_DRAIN_ROUNDS {
            let outcome = self.poll(proc_root, now, DRAIN_ROOM);
            if outcome.frames.is_empty() {
                break;
            }
            discarded += outcome.frames.len();
        }
        discarded
    }

    /// Reads at most `room` frames. Opens the session once a log file exists in the capture
    /// directory.
    pub fn poll(&mut self, proc_root: &std::path::Path, now: Instant, room: usize) -> PollOutcome {
        let alive = live_process(proc_root, self.launch.record.pid).is_some_and(|live| {
            live.start_ticks == self.launch.record.start_ticks && live.uid == self.uid
        });
        if !alive {
            self.publish(|status| status.healthy = false);
            return PollOutcome {
                frames: Vec::new(),
                gone: true,
            };
        }
        if self.session.is_none()
            && let Some(name) = newest_frame_log(&self.launch.dir)
            && let Ok(dir) = ConfinedDir::open(&self.launch.dir, self.uid)
        {
            self.session = Some(MangoHudTailSession::new(dir, &name, self.config, now));
        }
        let mut frames = Vec::new();
        let Some(session) = self.session.as_mut() else {
            return PollOutcome {
                frames,
                gone: false,
            };
        };
        let state = session.poll(now, room, &mut frames);
        let owner = session.identity().map(|identity| identity.uid);
        let produced = !frames.is_empty();
        self.publish(|status| {
            status.claim.file_owner_uid = owner;
            status.healthy = !matches!(state, TailState::Failed(_));
            if produced {
                status.last_frame_at = Some(now);
            }
        });
        PollOutcome {
            frames,
            gone: false,
        }
    }
}

impl std::fmt::Debug for ManagedFrameSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedFrameSource")
            .field("launch_pid", &self.launch.record.pid)
            .field("session_open", &self.session.is_some())
            .finish()
    }
}
