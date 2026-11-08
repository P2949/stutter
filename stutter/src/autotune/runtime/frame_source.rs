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
    pub(super) fn poll_frame_source(&mut self) {
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
            self.discover_frame_source(&root, proc_root, now);
        }
        let Some(source) = self.managed_source.as_mut() else {
            return;
        };
        let outcome = source.poll(proc_root, now, FRAMES_PER_POLL);
        if outcome.gone {
            self.managed_source = None;
            self.frame_source = None;
            self.controller.window.clear_frames();
            return;
        }
        for frame in outcome.frames {
            // A new epoch (rotation, truncation, replacement) never mixes with the old frames.
            if frame.epoch != self.frame_epoch {
                self.frame_epoch = frame.epoch;
                self.controller.window.clear_frames();
            }
            self.controller.window.push_frame(frame.event);
        }
    }

    fn discover_frame_source(&mut self, root: &Path, proc_root: &Path, now: Instant) {
        if self
            .last_frame_discovery
            .is_some_and(|at| now.duration_since(at) < DISCOVERY_INTERVAL)
        {
            return;
        }
        self.last_frame_discovery = Some(now);
        let Some(identity) = self.last_observation.workload_identity.clone() else {
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
        self.managed_source = Some(source);
    }
}
