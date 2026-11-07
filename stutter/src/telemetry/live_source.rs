//! The live state of one frame source, and its evaluation against the current target.
//!
//! A source owner (the tail driver) keeps a shared [`FrameSourceStatus`]. The autotune runtime
//! evaluates it against the target's identity at every tick, so a replaced process, a focus swap
//! or a stale or lossy stream withdraws frame-primary trust immediately, never later.
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::{
    frame_source::{Attribution, MeasurementStage, SourceClaim, SourceObservation, attribute},
    identity::WorkloadKey,
    proc_view::live_process,
    quality::StreamQuality,
};
use crate::autotune::observation::WorkloadIdentity;

/// What the source owner knows and publishes.
#[derive(Clone, Debug)]
pub struct FrameSourceStatus {
    pub claim: SourceClaim,
    pub last_frame_at: Option<Instant>,
    /// Frames lost between the source and the window.
    pub dropped_frames: u64,
    /// Sequence gaps with no known cause.
    pub unexplained_gaps: u64,
    pub healthy: bool,
    /// Other candidate games this source could equally belong to, as seen by the owner.
    pub competing_targets: usize,
    pub alignment_uncertainty_ms: f64,
}

impl FrameSourceStatus {
    pub fn new(claim: SourceClaim) -> Self {
        Self {
            claim,
            last_frame_at: None,
            dropped_frames: 0,
            unexplained_gaps: 0,
            healthy: true,
            competing_targets: 0,
            alignment_uncertainty_ms: 0.0,
        }
    }
}

pub type SharedFrameSource = Arc<Mutex<FrameSourceStatus>>;

/// Where the target stands right now, from the runtime.
pub struct TargetView<'a> {
    pub identity: &'a WorkloadIdentity,
    pub boot_id: &'a str,
    pub uid: u32,
    pub focused: bool,
}

fn unattributed(stage: MeasurementStage, status: &FrameSourceStatus) -> StreamQuality {
    StreamQuality {
        attribution: Attribution::ObservedUnattributed,
        stage,
        unexplained_sequence_gaps: status.unexplained_gaps,
        dropped_frames: status.dropped_frames,
        alignment_uncertainty_ms: status.alignment_uncertainty_ms,
        source_healthy: status.healthy,
    }
}

/// The quality of the source for this target at `now`. Any identity gap or denial yields a
/// non-direct attribution, so the result can only lower trust, never raise it by assumption.
pub fn evaluate(
    status: &FrameSourceStatus,
    target: &TargetView<'_>,
    proc_root: &Path,
    now: Instant,
) -> StreamQuality {
    let stage = status.claim.stage;
    let Ok(key) = WorkloadKey::from_identity(target.identity, target.boot_id, target.uid) else {
        return unattributed(stage, status);
    };
    // The target itself must still be the same process, or the whole identity is stale.
    let target_alive = live_process(proc_root, key.root_pid)
        .is_some_and(|live| live.start_ticks == key.start_ticks && live.uid == key.uid);
    if !target_alive {
        return unattributed(stage, status);
    }
    let age = status.last_frame_at.map_or(u64::MAX, |at| {
        now.saturating_duration_since(at)
            .as_millis()
            .min(u128::from(u64::MAX)) as u64
    });
    let observation = SourceObservation {
        target: key,
        target_focused: target.focused,
        producer: status
            .claim
            .producer_pid
            .and_then(|pid| live_process(proc_root, pid)),
        competing_targets: status.competing_targets,
        source_age_ms: age,
    };
    let attribution =
        attribute(&status.claim, stage, &observation).unwrap_or(Attribution::ObservedUnattributed);
    StreamQuality {
        attribution,
        ..unattributed(stage, status)
    }
}

/// Idle time after which a source with no frames is considered gone.
pub const SOURCE_IDLE_LIMIT: Duration = Duration::from_secs(5);
