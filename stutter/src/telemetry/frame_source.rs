//! Attribution of a frame source to a target workload.
use serde::{Deserialize, Serialize};

use super::identity::WorkloadKey;

/// A source is older than this when its last observation is considered stale.
pub const MAX_SOURCE_AGE_MS: u64 = 5_000;

/// Application frames, Gamescope presentation, Wayland presentation and KMS flips are different
/// measurement stages; one is never relabeled as another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MeasurementStage {
    ApplicationFrame,
    GamescopePresentation,
    WaylandPresentation,
    KmsFlip,
}

/// How far a source is trusted, weakest first. Only `ValidatedDirect` may be frame-primary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Attribution {
    Unavailable,
    ObservedUnattributed,
    Correlated,
    LaunchBound,
    ValidatedDirect,
}

/// What a source says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceClaim {
    pub stage: MeasurementStage,
    pub producer_pid: Option<u32>,
    pub producer_start_ticks: Option<u64>,
    /// Owner of the log file or socket, from its opened descriptor.
    pub file_owner_uid: Option<u32>,
    /// A launcher-supplied correlation identifier matched. Correlation only, never authority.
    pub launch_id_matched: bool,
}

/// A live process, as read now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveProcess {
    pub start_ticks: u64,
    pub uid: u32,
    /// Ancestors as `(pid, start_ticks)`, nearest first; includes re-executed and Proton parents.
    pub ancestors: Vec<(u32, u64)>,
}

/// Everything observed about the target and the source at decision time.
#[derive(Clone, Debug)]
pub struct SourceObservation {
    pub target: WorkloadKey,
    pub target_focused: bool,
    /// The claimed producer's live record, `None` when it is gone.
    pub producer: Option<LiveProcess>,
    /// Other candidate workloads this source could equally belong to.
    pub competing_targets: usize,
    pub source_age_ms: u64,
    /// The target root's ancestors as `(pid, start_ticks)`, nearest first.
    pub target_ancestors: Vec<(u32, u64)>,
}

/// Why a source cannot be attributed to the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceDenial {
    StageMismatch,
    WrongUser,
    ProducerGone,
    PidReused,
    NotInTargetTree,
    AmbiguousTargets,
    TargetNotFocused,
    StaleSource,
}

/// Attributes `claim` to the target for `wanted` stage, or says exactly why not.
pub fn attribute(
    claim: &SourceClaim,
    wanted: MeasurementStage,
    observed: &SourceObservation,
) -> Result<Attribution, SourceDenial> {
    if claim.stage != wanted {
        return Err(SourceDenial::StageMismatch);
    }
    let Some(pid) = claim.producer_pid else {
        return Ok(Attribution::ObservedUnattributed);
    };
    if claim
        .file_owner_uid
        .is_some_and(|uid| uid != observed.target.uid)
    {
        return Err(SourceDenial::WrongUser);
    }
    let live = observed
        .producer
        .as_ref()
        .ok_or(SourceDenial::ProducerGone)?;
    if live.uid != observed.target.uid {
        return Err(SourceDenial::WrongUser);
    }
    if claim
        .producer_start_ticks
        .is_none_or(|ticks| ticks != live.start_ticks)
    {
        return Err(SourceDenial::PidReused);
    }
    let is_target =
        pid == observed.target.root_pid && live.start_ticks == observed.target.start_ticks;
    let under_target = live.ancestors.iter().any(|&(ancestor, ticks)| {
        ancestor == observed.target.root_pid && ticks == observed.target.start_ticks
    });
    // The launcher may be the target or sit above it, and the renderer may sit below either: both
    // belong to one launch tree. Anything outside that relation is a different process.
    let target_under_producer = observed
        .target_ancestors
        .iter()
        .any(|&(ancestor, ticks)| ancestor == pid && Some(ticks) == claim.producer_start_ticks);
    if !is_target && !under_target && !target_under_producer {
        return Err(SourceDenial::NotInTargetTree);
    }
    if observed.competing_targets > 0 {
        return Err(SourceDenial::AmbiguousTargets);
    }
    if !observed.target_focused {
        return Err(SourceDenial::TargetNotFocused);
    }
    if observed.source_age_ms > MAX_SOURCE_AGE_MS {
        return Err(SourceDenial::StaleSource);
    }
    // Live identity, ancestry, focus and freshness all hold. Direct trust additionally needs the
    // file owner to have been verified; a launch id alone only binds, it never validates.
    if claim.file_owner_uid == Some(observed.target.uid) {
        Ok(Attribution::ValidatedDirect)
    } else if claim.launch_id_matched {
        Ok(Attribution::LaunchBound)
    } else {
        Ok(Attribution::Correlated)
    }
}
