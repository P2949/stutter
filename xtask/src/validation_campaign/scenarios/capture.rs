//! When a capture may enter the calibration corpus, and when the campaign stops trying. A run
//! whose intervention did not establish itself is a failed attempt: it never counts toward the
//! requested captures, and a bounded retry budget turns persistent failure into an honest
//! `BlockedResource` instead of a padded corpus.
use super::model::{ControlType, InterventionKind, ScenarioSpec, Target};

/// Result of the *independent* telemetry check of one applied intervention.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionReport {
    pub kind: InterventionKind,
    pub verified: bool,
    pub detail: String,
}

/// Everything known about a finished capture attempt, gathered from tools and telemetry, never
/// from Stutter's diagnosis output.
#[derive(Clone, Debug, Default)]
pub struct CaptureEvidence {
    pub target_identity_valid: bool,
    pub recording_valid: bool,
    /// Dropped events other than coalesced wakeups.
    pub drops: u64,
    pub replaced_wakeups: u64,
    pub block_collisions: u64,
    pub write_errors: u64,
    pub interventions: Vec<InterventionReport>,
    /// Source/campaign fingerprint still matches (HEAD, binaries, manifest).
    pub fingerprint_matches: bool,
    pub capture_id_unique: bool,
    pub sanitizer_ok: bool,
    pub fixture_valid: bool,
    pub frame_samples: u64,
    /// Why the target did not stay where the experiment placed it; `None` when it did (or the
    /// scenario does not constrain placement).
    pub placement_failure: Option<String>,
}

/// All acceptance conditions; `Err` lists every reason the attempt cannot count.
pub fn accept(spec: &ScenarioSpec, evidence: &CaptureEvidence) -> Result<(), Vec<String>> {
    let mut reasons = Vec::new();
    let mut require = |ok: bool, reason: &str| {
        if !ok {
            reasons.push(reason.to_owned());
        }
    };
    require(
        evidence.target_identity_valid,
        "target identity (pid, starttime) is not valid",
    );
    require(evidence.recording_valid, "the recording does not validate");
    require(evidence.drops == 0, "the recording dropped events");
    require(
        evidence.replaced_wakeups <= spec.accepted_wakeup_coalescing,
        &format!(
            "{} wakeup records were coalesced, above the scenario's {}",
            evidence.replaced_wakeups, spec.accepted_wakeup_coalescing
        ),
    );
    require(
        evidence.block_collisions <= spec.accepted_block_collisions,
        &format!(
            "{} block-I/O correlation keys collided, above the scenario's {}",
            evidence.block_collisions, spec.accepted_block_collisions
        ),
    );
    require(evidence.write_errors == 0, "the recording had write errors");
    if let Some(reason) = &evidence.placement_failure {
        require(false, &format!("target placement not verified: {reason}"));
    }
    require(
        evidence.fingerprint_matches,
        "the source/campaign fingerprint changed",
    );
    require(evidence.capture_id_unique, "the capture id is not unique");
    require(
        evidence.sanitizer_ok,
        "the sanitizer rejected the recording",
    );
    require(
        evidence.fixture_valid,
        "the generated fixture does not validate",
    );
    require(
        !matches!(spec.target, Target::VkmarkLight | Target::VkmarkGpuHeavy)
            || evidence.frame_samples > 0,
        "a vkmark target must produce a non-empty frame stream",
    );
    for expected in &spec.interventions {
        let verified = evidence
            .interventions
            .iter()
            .any(|report| report.kind == *expected && report.verified);
        let detail = evidence
            .interventions
            .iter()
            .find(|report| report.kind == *expected)
            .map(|report| report.detail.as_str())
            .filter(|detail| !detail.is_empty());
        require(
            verified,
            &format!(
                "intervention {expected:?} was not independently verified{}",
                detail.map_or_else(String::new, |d| format!(": {d}"))
            ),
        );
    }
    // A control must not carry an intervention, and an unexpected one invalidates the capture.
    for report in &evidence.interventions {
        require(
            spec.interventions.contains(&report.kind),
            &format!("unexpected intervention {:?} was active", report.kind),
        );
    }
    if spec.control == ControlType::NegativeControl {
        require(
            spec.interventions.is_empty(),
            "a negative control declares an intervention",
        );
    }
    if reasons.is_empty() {
        Ok(())
    } else {
        Err(reasons)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NextAction {
    /// Captures are still missing and the retry budget remains.
    Capture,
    Complete,
    /// Too many failed attempts: stop with retained evidence.
    Exhausted {
        failed: usize,
        limit: usize,
    },
}

/// Failed attempts tolerated per scenario: `failed_per_capture` for each required capture.
pub fn failure_limit(failed_per_capture: usize, need: usize) -> usize {
    failed_per_capture.saturating_mul(need)
}

pub fn next_action(
    have: usize,
    need: usize,
    failed: usize,
    failed_per_capture: usize,
) -> NextAction {
    if have >= need {
        return NextAction::Complete;
    }
    let limit = failure_limit(failed_per_capture, need);
    if failed >= limit {
        return NextAction::Exhausted { failed, limit };
    }
    NextAction::Capture
}
