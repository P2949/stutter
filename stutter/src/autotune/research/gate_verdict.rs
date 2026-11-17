//! One verdict per PLAN gate (G0-G9), each from its own evidence and each naming exactly what is
//! missing. A campaign verification can only attest what a campaign contains: the gates whose
//! evidence lives elsewhere (a host baseline, the lease and IPC test receipts, a resident telemetry
//! receipt, a fault matrix, a deployment receipt) are reported `accepted: false` with a blocker that
//! says so, never inferred from the studies. A health incident blocks every gate that needs a
//! running game.

use serde::Serialize;

use super::{
    child_ledger::Disposition,
    child_study::{Profile, Stage, StudyResult},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GateVerdict {
    pub gate: &'static str,
    pub accepted: bool,
    /// Why it is not accepted (empty exactly when it is).
    pub blockers: Vec<&'static str>,
}

/// What a read-only re-reading of the host found, compared with what the campaign froze (G0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostFacts {
    pub same_boot: bool,
    pub build_id_matches: bool,
    pub game_executable_matches: bool,
    /// Steam indicates no pending update of the installation.
    pub installation_settled: bool,
    /// Every Proton identity the campaign froze is still installed, eligible and unchanged.
    pub proton_identities_match: bool,
    /// The Steam client is the one the campaign froze (branch and runtime identity).
    pub steam_client_matches: bool,
    /// The signed-in account still has a launch record for the game (the same evidence the
    /// production host requires before it will launch).
    pub entitlement_verified: bool,
}

/// What a read-only inspection of the campaign's lease found (G1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeaseFacts {
    pub chain_valid: bool,
    pub transitions_valid: bool,
    pub header_matches_intent: bool,
    /// The lease ended (completed, blocked or aborted) instead of stopping mid-way.
    pub terminal: bool,
    /// The lease was created for the campaign being verified (its intent digest).
    pub for_this_campaign: bool,
    /// A live process holds the lease's exclusive lock (`None`: the lock table could not be read).
    pub owner_live: Option<bool>,
    /// What the header recorded of the admission (an identified peer, intact facts, inside the window).
    pub admission: super::lease_inspect::Admission,
}

/// What the gates are computed from.
pub struct GateInputs<'a> {
    pub raw_files_verified: bool,
    pub loaded_runtime_qualified: bool,
    pub child_studies: &'a [Disposition],
    pub child_reports_verified: bool,
    pub restoration_verified: bool,
    pub health_blocked: bool,
    /// (attempts with an eligible, game-bound frame-source receipt, completed attempts); `None` when
    /// no attempt completed.
    pub telemetry: Option<(u32, u32)>,
    /// Whether every holdout reproduced its confirmation (`None`: no holdout was judged).
    pub replication: Option<bool>,
    /// G6: whether a physical fault-matrix receipt for this binary attests (`fault_matrix`), or why not.
    pub fault_matrix: Result<(), &'static str>,
    /// G2: the player's saves and cloud copy are unchanged since before the campaign (`protected_state`).
    pub protected_state: Result<(), &'static str>,
    /// A deploy receipt names the very binary doing this verification, runs the expected pin and has
    /// every unit active (`deploy_receipt::deployment_attested`).
    pub deployment_attested: bool,
    /// The lease inspection, `None` when the lease directory could not be inspected.
    pub lease: Option<LeaseFacts>,
    /// The host baseline comparison, `None` when the installation could not be read.
    pub host: Option<HostFacts>,
}

/// `Some(true)` when there is at least one study of the kind and every one is qualified, `Some(false)`
/// when some is not, `None` when the plan has none.
fn qualified(studies: &[Disposition], kind: fn(&Stage) -> bool) -> Option<bool> {
    let mut matching = studies.iter().filter(|study| kind(&study.stage)).peekable();
    matching.peek()?;
    Some(matching.all(|study| study.result == Some(StudyResult::Qualified)))
}

fn gate(gate: &'static str, blockers: Vec<&'static str>) -> GateVerdict {
    GateVerdict {
        gate,
        accepted: blockers.is_empty(),
        blockers,
    }
}

/// Study blockers for one stage kind: its absence from the plan, or its not being qualified.
fn study_blockers(
    studies: &[Disposition],
    kind: fn(&Stage) -> bool,
    missing: &'static str,
    unqualified: &'static str,
) -> Vec<&'static str> {
    match qualified(studies, kind) {
        None => vec![missing],
        Some(false) => vec![unqualified],
        Some(true) => Vec::new(),
    }
}

/// As `study_blockers`, and the frozen plan's every performance profile must have such a study: a
/// CPU result alone does not stand for the CPU and GPU comparisons the plan asks for.
fn profile_blockers(
    studies: &[Disposition],
    kind: fn(&Stage) -> bool,
    missing: &'static str,
    unqualified: &'static str,
) -> Vec<&'static str> {
    let mut blockers = study_blockers(studies, kind, missing, unqualified);
    let covered = |profile| {
        studies
            .iter()
            .any(|study| study.profile == profile && kind(&study.stage))
    };
    if !blockers.contains(&missing) && !(covered(Profile::Cpu) && covered(Profile::Gpu)) {
        blockers.push("MissingProfileStudy");
    }
    blockers
}

/// G0 from the host comparison: the host is still the one the campaign froze. Whether the original
/// capture was right is not claimed, only that nothing has moved away from it.
fn host_blockers(host: Option<HostFacts>) -> Vec<&'static str> {
    let Some(host) = host else {
        return vec!["HostNotInspected"];
    };
    let mut blockers = Vec::new();
    if !host.same_boot {
        blockers.push("HostRebootedSinceCampaign");
    }
    if !host.build_id_matches {
        blockers.push("GameBuildChanged");
    }
    if !host.game_executable_matches {
        blockers.push("GameExecutableChanged");
    }
    if !host.installation_settled {
        blockers.push("InstallationNotSettled");
    }
    if !host.proton_identities_match {
        blockers.push("ProtonIdentityChanged");
    }
    if !host.steam_client_matches {
        blockers.push("SteamClientChanged");
    }
    if !host.entitlement_verified {
        blockers.push("EntitlementNotVerified");
    }
    blockers
}

/// G1 from the lease inspection: the sealed records hold together, end in a terminal phase and belong
/// to this campaign. The authenticated-IPC and single-owner properties are tested elsewhere and are
/// not claimed here.
fn lease_blockers(lease: Option<LeaseFacts>) -> Vec<&'static str> {
    let Some(lease) = lease else {
        return vec!["LeaseNotInspected"];
    };
    let mut blockers = Vec::new();
    if !lease.chain_valid {
        blockers.push("LeaseChainBroken");
    }
    if !lease.transitions_valid {
        blockers.push("LeaseTransitionsInvalid");
    }
    if !lease.header_matches_intent {
        blockers.push("LeaseHeaderMismatch");
    }
    if !lease.for_this_campaign {
        blockers.push("LeaseForAnotherCampaign");
    }
    if !lease.admission.peer_identified {
        blockers.push("LeaseAdmissionPeerUnidentified");
    }
    if !lease.admission.facts_consistent {
        blockers.push("LeaseFactsInconsistent");
    }
    if !lease.admission.admitted_in_window {
        blockers.push("LeaseAdmittedOutsideIntentWindow");
    }
    match lease.owner_live {
        Some(false) => {}
        Some(true) => blockers.push("LeaseOwnerStillLive"),
        None => blockers.push("LeaseOwnerUnknown"),
    }
    if !lease.terminal {
        blockers.push("LeaseNotSettled");
    }
    blockers
}

pub fn gates(inputs: &GateInputs<'_>) -> Vec<GateVerdict> {
    let studies = inputs.child_studies;
    let health = |mut blockers: Vec<&'static str>| {
        if inputs.health_blocked {
            blockers.push("GpuHealthIncident");
        }
        blockers
    };
    // A lease is about records, not a running game: a health incident does not change it.
    let health_free = |blockers: Vec<&'static str>| blockers;
    let screen = |stage: &Stage| matches!(stage, Stage::Screen);
    let comparison = |stage: &Stage| {
        matches!(
            stage,
            Stage::Screen | Stage::Pilot | Stage::Confirm | Stage::Holdout
        )
    };
    let holdout = |stage: &Stage| matches!(stage, Stage::Holdout);
    let action = |stage: &Stage| matches!(stage, Stage::Action(_));
    let feature = |stage: &Stage| matches!(stage, Stage::Feature(_));

    // G2: a genuine Steam launch of the game with raw native evidence that replays, run under each
    // arm's own installation, across a qualified screen.
    let mut g2 = profile_blockers(studies, screen, "NoScreenStudy", "ScreenNotQualified");
    if !inputs.raw_files_verified {
        g2.push("RawFilesUnverified");
    }
    if !inputs.loaded_runtime_qualified {
        g2.push("LoadedRuntimeUnproven");
    }
    // "Original data intact": the saves and cloud copy fingerprinted before the first launch are the
    // same now. Not recorded, unreadable or changed each block, and none is read as unchanged.
    if let Err(reason) = inputs.protected_state {
        g2.push(reason);
    }

    // G4: the resident funnel's comparison stages all qualified, each child independently verified.
    let mut g4 = profile_blockers(
        studies,
        comparison,
        "NoComparisonStudies",
        "ComparisonNotQualified",
    );
    if !inputs.child_reports_verified {
        g4.push("ChildReportsUnverified");
    }

    // G5: a real Supported action, with restoration proven by a verifier independent of its token.
    let mut g5 = study_blockers(studies, action, "NoActionStudy", "ActionNotQualified");
    if !inputs.restoration_verified {
        g5.push("RestorationUnverified");
    }

    // G3: what the resident controller itself saw. Every completed attempt needs a receipt showing a
    // healthy, lossless frame source bound to the attested game with enough frames and a bounded clock.
    let g3 = match inputs.telemetry {
        None => vec!["NoTelemetryReceipts"],
        Some((eligible, completed)) if eligible < completed => vec!["FrameSourceNotEligible"],
        Some(_) => Vec::new(),
    };

    // G7: effective feature activation per factor (a qualified feature study carries that proof).
    let g7 = study_blockers(studies, feature, "NoFeatureStudy", "FeatureNotQualified");

    // G9: the held-out repetition qualified on top of everything a result rests on, with the
    // system restored and no health incident.
    let mut g9 = profile_blockers(studies, holdout, "NoHoldoutStudy", "HoldoutNotQualified");
    // A sound holdout is not a replication: it must reach its confirmation's conclusion. A
    // contradiction, or an inconclusive repetition, reproduces nothing.
    if inputs.replication == Some(false) {
        g9.push("HoldoutDoesNotReplicate");
    }
    for (name, blockers) in [("G2", &g2), ("G4", &g4), ("G5", &g5), ("G7", &g7)] {
        if !blockers.is_empty() {
            g9.push(match name {
                "G2" => "G2NotAccepted",
                "G4" => "G4NotAccepted",
                "G5" => "G5NotAccepted",
                _ => "G7NotAccepted",
            });
        }
    }

    vec![
        gate("G0", host_blockers(inputs.host)),
        gate("G1", health_free(lease_blockers(inputs.lease))),
        gate("G2", health(g2)),
        gate("G3", health(g3)),
        gate("G4", health(g4)),
        gate("G5", health(g5)),
        // Only a physical matrix for this very binary stands for G6; fixtures never do.
        gate("G6", inputs.fault_matrix.err().into_iter().collect()),
        gate("G7", health(g7)),
        gate(
            "G8",
            if inputs.deployment_attested {
                Vec::new()
            } else {
                vec!["DeploymentNotAttested"]
            },
        ),
        gate("G9", health(g9)),
    ]
}

#[cfg(test)]
#[path = "gate_verdict_tests.rs"]
mod tests;
