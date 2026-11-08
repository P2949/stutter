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
/// How often the launch tree and capture directory are re-examined for competing producers.
const COMPETITION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
/// A log written this recently counts as live.
const LIVE_LOG_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

pub struct PollOutcome {
    pub frames: Vec<TailedFrame>,
    /// The launched process no longer exists or is not the one recorded.
    pub gone: bool,
}

/// What a phase-boundary drain achieved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drain {
    pub discarded: usize,
    /// The log was read to its end: nothing produced before the boundary can reach the next phase.
    /// `false` means the bound ran out first, and the next window is not trustworthy.
    pub proven: bool,
}

pub struct ManagedFrameSource {
    launch: DiscoveredLaunch,
    uid: u32,
    session: Option<MangoHudTailSession>,
    status: SharedFrameSource,
    config: TailConfig,
    /// The log file the session is following; a different newest log starts a new generation.
    log_name: Option<String>,
    /// Added to every session epoch so a new log never reuses an old epoch number.
    generation: u64,
    /// Rejected and oversize rows already counted when the current loss window began.
    loss_baseline: u64,
    /// Frames read but not placeable on the controller's clock, in the current loss window.
    unplaced: u64,
    /// A phase-boundary drain ran out of bound: completeness is not known until the next one.
    boundary_unproven: bool,
    last_competition_check: Option<Instant>,
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
            log_name: None,
            generation: 0,
            loss_baseline: 0,
            unplaced: 0,
            boundary_unproven: false,
            last_competition_check: None,
        }
    }

    /// The verified launch this source reads.
    pub fn launch(&self) -> &DiscoveredLaunch {
        &self.launch
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

    fn session_loss(&self) -> u64 {
        self.session.as_ref().map_or(0, |session| {
            let counters = session.counters();
            counters
                .oversize_lines_discarded
                .saturating_add(counters.rejected_lines)
        })
    }

    /// Publishes the loss in the current window: rows the parser rejected or discarded as
    /// oversize, plus frames that could not be placed on the controller's clock, and an unexplained
    /// gap while the last phase boundary is unproven.
    fn publish_loss(&self) {
        let dropped = self
            .session_loss()
            .saturating_sub(self.loss_baseline)
            .saturating_add(self.unplaced);
        let gaps = u64::from(self.boundary_unproven);
        self.publish(|status| {
            status.dropped_frames = dropped;
            status.unexplained_gaps = gaps;
        });
    }

    /// Publishes the measured clock alignment (`NaN` when unknown) and counts frames that could
    /// not be placed on the controller's clock as lost.
    pub fn record_alignment(&mut self, uncertainty_ms: f64, unplaced_frames: u64) {
        self.unplaced = self.unplaced.saturating_add(unplaced_frames);
        self.publish(|status| status.alignment_uncertainty_ms = uncertainty_ms);
        self.publish_loss();
    }

    /// Publishes how old the newest frame is on the controller's clock. `None` (unknown) leaves
    /// the previous value, which then simply ages; an unknown age never reads as fresh.
    pub fn record_frame_age(&self, now: Instant, age: Option<std::time::Duration>) {
        if let Some(age) = age {
            self.publish(|status| status.last_frame_at = now.checked_sub(age));
        }
    }

    /// Starts a fresh loss window: what was lost before this point does not taint the next one.
    pub fn reset_loss(&mut self) {
        self.loss_baseline = self.session_loss();
        self.unplaced = 0;
        self.publish_loss();
    }

    /// Reads and discards every frame already in the log, so a phase boundary does not let frames
    /// produced before it land in the next phase's window. Bounded: at most `MAX_DRAIN_ROUNDS`
    /// polls of `DRAIN_ROOM` frames. The boundary is *proven* only if a poll found the log empty;
    /// otherwise the next window is marked untrusted instead of silently assumed clean.
    pub fn discard_pending(&mut self, proc_root: &std::path::Path, now: Instant) -> Drain {
        let mut discarded = 0;
        let mut proven = false;
        for _ in 0..MAX_DRAIN_ROUNDS {
            let outcome = self.poll(proc_root, now, DRAIN_ROOM);
            if outcome.gone || outcome.frames.is_empty() {
                proven = !outcome.gone;
                break;
            }
            discarded += outcome.frames.len();
        }
        self.boundary_unproven = !proven;
        if proven {
            self.reset_loss();
        } else {
            self.publish_loss();
        }
        Drain { discarded, proven }
    }

    /// Looks for competing producers: more than one MangoHud-injected renderer in the launch tree,
    /// or more than one live log in the capture directory. Either makes the source ambiguous (it
    /// cannot be said which process wrote which rows); no renderer found at all means the producer
    /// could not be established. Both count as a competing target and deny direct attribution.
    fn check_competition(&mut self, proc_root: &std::path::Path, now: Instant) {
        if self
            .last_competition_check
            .is_some_and(|at| now.saturating_duration_since(at) < COMPETITION_INTERVAL)
        {
            return;
        }
        self.last_competition_check = Some(now);
        let renderers = super::renderers::count_mangohud_renderers(
            proc_root,
            self.launch.record.pid,
            self.launch.record.start_ticks,
            self.uid,
        );
        let logs = super::renderers::live_log_count(&self.launch.dir, LIVE_LOG_WINDOW);
        let renderer_competition = if renderers == 0 { 1 } else { renderers - 1 };
        let log_competition = logs.saturating_sub(1);
        let competing = renderer_competition + log_competition;
        self.publish(|status| status.competing_targets = competing);
    }

    /// Reads at most `room` frames. Opens the session once a log file exists in the capture
    /// directory, and follows a newer log of the same verified launch as a new generation.
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
        self.check_competition(proc_root, now);
        let newest = newest_frame_log(&self.launch.dir);
        if let Some(name) = newest
            && self.log_name.as_deref() != Some(name.as_str())
            && let Ok(dir) = ConfinedDir::open(&self.launch.dir, self.uid)
        {
            // A new log of the same launch is a new source generation: nothing carries across.
            if self.session.is_some() {
                self.generation += 1;
            }
            self.session = Some(MangoHudTailSession::new(dir, &name, self.config, now));
            self.log_name = Some(name);
            self.loss_baseline = 0;
            self.unplaced = 0;
            self.boundary_unproven = false;
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
        for frame in &mut frames {
            frame.epoch = frame.epoch.wrapping_add(self.generation << 32);
        }
        self.publish(|status| {
            status.claim.file_owner_uid = owner;
            status.healthy = !matches!(state, TailState::Failed(_));
        });
        self.publish_loss();
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
