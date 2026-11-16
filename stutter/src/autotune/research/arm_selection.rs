//! Choosing the two arms of a pilot, confirmation, feature or action study from the screen's own
//! sealed results (PLAN section 6.3). The control is the stable baseline (Valve's Proton when the
//! campaign authorized it, otherwise the first authorized tool) and the candidate is the other
//! arm with the highest mean native FPS in the screen. It is a deterministic function of sealed
//! evidence, so a verifier reproduces it, and ties break by the tool order of the campaign.

use std::{collections::BTreeMap, fs::File, os::fd::AsFd};

use super::{
    ProtonSource, ResearchIntentV1,
    child_study::{ChildStudy, Profile, Stage, plan},
    report_store::read_evidence,
};

/// Valve's stable build when authorized, otherwise the campaign's first tool.
pub fn control_of(intent: &ResearchIntentV1) -> Option<ProtonSource> {
    let tools: Vec<ProtonSource> = intent.tools.iter().map(|tool| tool.source).collect();
    tools
        .contains(&ProtonSource::ValveStable)
        .then_some(ProtonSource::ValveStable)
        .or_else(|| tools.first().copied())
}

/// The control and the best non-control arm by mean FPS (campaign tool order breaks ties).
pub fn select_pair(
    intent: &ResearchIntentV1,
    fps: &BTreeMap<ProtonSource, Vec<f64>>,
) -> Option<[ProtonSource; 2]> {
    let control = control_of(intent)?;
    let mut best: Option<(ProtonSource, f64)> = None;
    for tool in &intent.tools {
        let arm = tool.source;
        if arm == control {
            continue;
        }
        let Some(runs) = fps.get(&arm).filter(|runs| !runs.is_empty()) else {
            continue;
        };
        let mean = runs.iter().sum::<f64>() / runs.len() as f64;
        if !mean.is_finite() {
            continue;
        }
        if best.is_none_or(|(_, top)| mean > top) {
            best = Some((arm, mean));
        }
    }
    // The control must itself have been measured, or there is nothing to compare against.
    fps.get(&control).filter(|runs| !runs.is_empty())?;
    best.map(|(candidate, _)| [control, candidate])
}

/// Whether a factor has a documented environment control on this Proton build. A fork-local feature
/// (an FSR or frame-generation switch only some forks carry) cannot be evaluated on a build that has
/// none, whatever its performance rank.
pub fn eligible_for(source: ProtonSource, factor: super::ResearchFactor) -> bool {
    super::features::factor_env(source, factor, true).is_ok()
}

/// The one build a within-source study measures. An action study uses the stable control. A feature
/// study uses the highest-FPS measured build that actually has an environment control for its factor
/// (campaign tool order breaks ties); `None` when no measured build has one, which makes the study
/// blocked rather than run on a build that would refuse the factor.
pub fn select_treatment_source(
    intent: &ResearchIntentV1,
    study: &ChildStudy,
    fps: &BTreeMap<ProtonSource, Vec<f64>>,
) -> Option<ProtonSource> {
    let mean = |source: ProtonSource| -> Option<f64> {
        let runs = fps.get(&source).filter(|runs| !runs.is_empty())?;
        let mean = runs.iter().sum::<f64>() / runs.len() as f64;
        mean.is_finite().then_some(mean)
    };
    match study.stage {
        Stage::Action(_) => control_of(intent).filter(|control| mean(*control).is_some()),
        Stage::Feature(factor) => {
            let mut best: Option<(ProtonSource, f64)> = None;
            for tool in &intent.tools {
                let source = tool.source;
                if !eligible_for(source, factor) {
                    continue;
                }
                let Some(value) = mean(source) else { continue };
                if best.is_none_or(|(_, top)| value > top) {
                    best = Some((source, value));
                }
            }
            best.map(|(source, _)| source)
        }
        Stage::Screen | Stage::Pilot | Stage::Confirm => None,
    }
}

/// The screen a study descends from for its own profile (a pilot's or confirmation's predecessor
/// chain, or the CPU screen for the action and feature studies).
pub fn screen_of(intent: &ResearchIntentV1, study: &ChildStudy) -> anyhow::Result<ChildStudy> {
    let wanted = match study.stage {
        Stage::Action(_) | Stage::Feature(_) => Profile::Cpu,
        _ => study.profile,
    };
    plan(intent)?
        .into_iter()
        .find(|candidate| candidate.stage == Stage::Screen && candidate.profile == wanted)
        .ok_or_else(|| anyhow::anyhow!("the study has no screen to select its arms from"))
}

/// Each arm's native average FPS per completed run in a screen study's own sealed evidence.
pub fn screen_fps(
    campaign: &File,
    uid: u32,
    screen: &ChildStudy,
) -> anyhow::Result<BTreeMap<ProtonSource, Vec<f64>>> {
    let studies = File::from(crate::syscall::openat_dir_nofollow(
        campaign.as_fd(),
        "studies",
    )?);
    let dir = File::from(crate::syscall::openat_dir_nofollow(
        studies.as_fd(),
        &screen.study_sha256[..16],
    )?);
    // The selection stands on the screen only if its report is the deterministic function of its own
    // sealed journal and evidence; tampered or torn evidence selects nothing.
    super::report_store::load_and_verify(&dir, uid)?;
    let evidence = File::from(crate::syscall::openat_dir_nofollow(
        dir.as_fd(),
        "evidence",
    )?);
    let mut fps: BTreeMap<ProtonSource, Vec<f64>> = BTreeMap::new();
    for item in read_evidence(&evidence, uid)?.into_values() {
        if let Some(native) = item.native_completion.as_deref() {
            fps.entry(native.proton_source)
                .or_default()
                .push(native.metrics.average_fps);
        }
    }
    Ok(fps)
}

#[cfg(test)]
#[path = "arm_selection_tests.rs"]
mod tests;
