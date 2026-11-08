//! The opt-in bridge from `stutter game-run` capture directories to the runtime's frame window.
//!
//! Disabled unless `frame_capture_root` is configured, so source-less runnable-latency tuning is
//! unchanged. When enabled, the runtime looks for a verified launch whose process tree contains
//! the focused target, tails its MangoHud log through a confined session, feeds the frames into
//! the rolling window and lets `live_source::evaluate` decide trust every tick.
use std::{
    path::Path,
    time::{Duration, Instant},
};

use super::AutotuneRuntime;
use crate::{
    autotune::observation::WorkloadIdentity,
    telemetry::{
        identity::read_boot_id,
        launch_discovery::{DiscoveredLaunch, discover},
        managed_source::ManagedFrameSource,
        proc_view::live_process,
    },
};

/// How often a runtime without a source looks for one.
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(5);
/// Most frames moved into the window by one poll.
const FRAMES_PER_POLL: usize = 2048;

/// Whether `launch` and the target belong to one launch tree.
pub(super) fn launch_matches_target(
    launch: &DiscoveredLaunch,
    target: &WorkloadIdentity,
    proc_root: &Path,
) -> bool {
    let Some(target_ticks) = target.process_starttime_ticks else {
        return false;
    };
    let record = &launch.record;
    if record.pid == target.root_pid && record.start_ticks == target_ticks {
        return true;
    }
    if launch
        .live
        .ancestors
        .iter()
        .any(|&(pid, ticks)| pid == target.root_pid && ticks == target_ticks)
    {
        return true;
    }
    live_process(proc_root, target.root_pid).is_some_and(|live| {
        live.start_ticks == target_ticks
            && live
                .ancestors
                .iter()
                .any(|&(pid, ticks)| pid == record.pid && ticks == record.start_ticks)
    })
}

impl AutotuneRuntime {
    /// Finds, polls and retires the managed frame source. A no-op without a configured root.
    pub(super) fn poll_frame_source(&mut self, identity: Option<&WorkloadIdentity>) {
        let Some(root) = self
            .config
            .daemon_config
            .autotune
            .frame_capture_root
            .clone()
        else {
            return;
        };
        let proc_root = Path::new("/proc");
        let now = Instant::now();
        if self.managed_source.is_none() {
            self.discover_frame_source(&root, proc_root, now, identity);
        }
        let Some(source) = self.managed_source.as_mut() else {
            return;
        };
        let outcome = source.poll(proc_root, now, FRAMES_PER_POLL);
        if outcome.gone {
            self.managed_source = None;
            self.frame_source = None;
            self.frame_clock = crate::telemetry::frame_clock::FrameClock::default();
            self.controller.window.clear_frames();
            return;
        }
        // The game's clock is not the monitor's: every frame is mapped through a measured offset,
        // and the source's alignment uncertainty is whatever that measurement can bound (unknown
        // until it can bound it), never an assumed zero.
        let monitor_now = self.interval_anchor.map(|(elapsed_ms, arrived)| {
            elapsed_ms.saturating_add(
                u64::try_from(now.saturating_duration_since(arrived).as_millis())
                    .unwrap_or(u64::MAX),
            )
        });
        let aligned = self
            .frame_clock
            .align(now, monitor_now, FRAMES_PER_POLL, outcome.frames);
        let uncertainty = self.frame_clock.uncertainty_ms();
        source.record_alignment(uncertainty, aligned.unplaced);
        if aligned.epoch_changed {
            // A new epoch (rotation, truncation, replacement) never mixes with the old frames,
            // and what was lost in the old one does not taint the new one.
            source.reset_loss();
        }
        if aligned.epoch_changed || aligned.invalidate_window {
            // Frames mapped under an earlier alignment are not covered by this one. Clearing also
            // withdraws the attestation, which is re-derived from the source on this tick.
            self.controller.window.clear_frames();
        }
        let current = aligned.frames.last().map(|(epoch, _)| *epoch);
        let mut newest_mapped = None;
        for (epoch, event) in aligned.frames {
            if Some(epoch) != current {
                continue;
            }
            self.frame_epoch = epoch;
            newest_mapped = Some(event.elapsed_ms);
            self.controller.window.push_frame(event);
        }
        // Frame age is measured on the controller's clock from the frame's mapped time, never
        // from when it was read: an old backlog read now is old. It is only claimed when the
        // alignment itself is bounded, and an unknown age stays unknown (it ages, never refreshes).
        if let (Some(mapped), Some(now_ms), true) =
            (newest_mapped, monitor_now, uncertainty.is_finite())
        {
            let age_ms = now_ms.saturating_sub(mapped);
            source.record_frame_age(now, Some(Duration::from_millis(age_ms)));
        }
    }

    fn discover_frame_source(
        &mut self,
        root: &Path,
        proc_root: &Path,
        now: Instant,
        identity: Option<&WorkloadIdentity>,
    ) {
        if self
            .last_frame_discovery
            .is_some_and(|at| now.duration_since(at) < DISCOVERY_INTERVAL)
        {
            return;
        }
        self.last_frame_discovery = Some(now);
        let Some(identity) = identity.cloned() else {
            return;
        };
        let Ok(boot_id) = read_boot_id() else {
            return;
        };
        let uid = crate::syscall::geteuid();
        let Some(launch) = discover(root, uid, &boot_id, proc_root)
            .into_iter()
            .find(|launch| launch_matches_target(launch, &identity, proc_root))
        else {
            return;
        };
        let source = ManagedFrameSource::new(launch, uid);
        self.frame_source = Some(source.shared_status());
        self.frame_epoch = 0;
        self.frame_clock = crate::telemetry::frame_clock::FrameClock::default();
        self.managed_source = Some(source);
    }
}

impl AutotuneRuntime {
    /// Ends a measurement phase: clears the window and discards the frames already waiting in the
    /// source, so nothing produced before the boundary is scored in the next phase.
    pub(super) fn clear_measurement_window(&mut self) {
        self.controller.window.clear();
        if let Some(source) = self.managed_source.as_mut() {
            source.discard_pending(Path::new("/proc"), Instant::now());
        }
    }
}

impl AutotuneRuntime {
    /// Whether frame capture is configured at all; without it nothing here runs.
    pub(super) fn frame_capture_enabled(&self) -> bool {
        self.config
            .daemon_config
            .autotune
            .frame_capture_root
            .is_some()
    }

    /// Retires a source that does not belong to the target *as it is now*. A focus change, a
    /// replaced process or a lost identity ends the source and its attestation at once, before any
    /// frame from it can be read for, or attested against, the new target.
    pub(super) fn reconcile_frame_source(&mut self, identity: Option<&WorkloadIdentity>) {
        let Some(source) = self.managed_source.as_ref() else {
            return;
        };
        let belongs = identity.is_some_and(|identity| {
            launch_matches_target(source.launch(), identity, Path::new("/proc"))
        });
        if belongs {
            return;
        }
        self.managed_source = None;
        self.frame_source = None;
        self.frame_clock = crate::telemetry::frame_clock::FrameClock::default();
        self.frame_epoch = 0;
        self.controller.window.clear_frames();
    }
}
