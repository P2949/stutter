//! Choosing the two arms of a pilot, confirmation, feature or action study from the screen's own
//! sealed results (PLAN section 6.3). The control is the first viable preferred baseline (Valve
//! Stable, then Experimental, then the campaign's remaining tools) and the candidate is the other
//! arm with the highest mean native FPS in the screen. It is a deterministic function of sealed
//! evidence, so a verifier reproduces it, and ties break by the tool order of the campaign.

use std::{collections::BTreeMap, fs::File, os::fd::AsFd};

use super::{
    ProtonSource, ResearchIntentV1,
    child_study::{ChildStudy, Profile, Stage, plan},
    report_store::read_evidence,
};

fn mean_fps(fps: &BTreeMap<ProtonSource, Vec<f64>>, source: ProtonSource) -> Option<f64> {
    let runs = fps.get(&source).filter(|runs| !runs.is_empty())?;
    if !runs.iter().all(|value| value.is_finite() && *value > 0.0) {
        return None;
    }
    let mean = runs
        .iter()
        .map(|value| value / runs.len() as f64)
        .sum::<f64>();
    (mean.is_finite() && mean > 0.0).then_some(mean)
}

/// Screen results establish viability only; they do not rank builds or prove shader-cache warmth.
pub fn viable_sources(
    intent: &ResearchIntentV1,
    fps: &BTreeMap<ProtonSource, Vec<f64>>,
) -> Vec<ProtonSource> {
    intent
        .tools
        .iter()
        .map(|tool| tool.source)
        .filter(|source| mean_fps(fps, *source).is_some())
        .collect()
}

/// Choose a control from authorized tools with usable measurements, preferring Valve controls.
pub fn control_of(
    intent: &ResearchIntentV1,
    fps: &BTreeMap<ProtonSource, Vec<f64>>,
) -> Option<ProtonSource> {
    let viable = |source| {
        intent.tools.iter().any(|tool| tool.source == source) && mean_fps(fps, source).is_some()
    };
    [ProtonSource::ValveStable, ProtonSource::ValveExperimental]
        .into_iter()
        .find(|source| viable(*source))
        .or_else(|| {
            intent
                .tools
                .iter()
                .map(|tool| tool.source)
                .find(|source| viable(*source))
        })
}

/// The control and the best non-control arm by mean FPS (campaign tool order breaks ties).
pub fn select_pair(
    intent: &ResearchIntentV1,
    fps: &BTreeMap<ProtonSource, Vec<f64>>,
) -> Option<[ProtonSource; 2]> {
    let sources: Vec<_> = intent.tools.iter().map(|tool| tool.source).collect();
    select_pair_from_sources(&sources, fps)
}

pub(super) fn select_pair_from_sources(
    sources: &[ProtonSource],
    fps: &BTreeMap<ProtonSource, Vec<f64>>,
) -> Option<[ProtonSource; 2]> {
    let viable = |source| sources.contains(&source) && mean_fps(fps, source).is_some();
    let control = [ProtonSource::ValveStable, ProtonSource::ValveExperimental]
        .into_iter()
        .find(|source| viable(*source))
        .or_else(|| sources.iter().copied().find(|source| viable(*source)))?;
    let mut best: Option<(ProtonSource, f64)> = None;
    for &arm in sources {
        if arm == control {
            continue;
        }
        let Some(mean) = mean_fps(fps, arm) else {
            continue;
        };
        if best.is_none_or(|(_, top)| mean > top) {
            best = Some((arm, mean));
        }
    }
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
    let mean = |source: ProtonSource| mean_fps(fps, source);
    match study.stage {
        Stage::Action(_) => control_of(intent, fps),
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
        Stage::Screen | Stage::Pilot | Stage::Confirm | Stage::Holdout | Stage::MatchedCachyOs => {
            None
        }
    }
}

/// The screen a study descends from for its own profile (a pilot's or confirmation's predecessor
/// chain, or the CPU screen for the action and feature studies).
pub fn screen_of(intent: &ResearchIntentV1, study: &ChildStudy) -> anyhow::Result<ChildStudy> {
    let wanted = match study.stage {
        Stage::Action(_) | Stage::Feature(_) | Stage::MatchedCachyOs => Profile::Cpu,
        _ => study.profile,
    };
    plan(intent)?
        .into_iter()
        .find(|candidate| candidate.stage == Stage::Screen && candidate.profile == wanted)
        .ok_or_else(|| anyhow::anyhow!("the study has no screen to select its arms from"))
}

/// The pilot whose repeated whole runs select the confirmation and treatment sources.
pub fn pilot_of(intent: &ResearchIntentV1, study: &ChildStudy) -> anyhow::Result<ChildStudy> {
    let wanted = match study.stage {
        Stage::Action(_) | Stage::Feature(_) | Stage::MatchedCachyOs => Profile::Cpu,
        _ => study.profile,
    };
    plan(intent)?
        .into_iter()
        .find(|candidate| candidate.stage == Stage::Pilot && candidate.profile == wanted)
        .ok_or_else(|| anyhow::anyhow!("the study has no pilot to select its arms from"))
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
    screen_fps_in_studies(&studies, uid, screen)
}

pub fn screen_fps_in_studies(
    studies: &File,
    uid: u32,
    screen: &ChildStudy,
) -> anyhow::Result<BTreeMap<ProtonSource, Vec<f64>>> {
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
    let runs: Vec<_> = read_evidence(&evidence, uid)?
        .into_iter()
        .filter_map(|(slot, item)| {
            let native = item.native_completion.as_deref()?;
            Some((slot, native.proton_source, native.metrics.average_fps))
        })
        .collect();
    let mut fps = BTreeMap::new();
    for (_, source, value) in runs {
        fps.entry(source).or_insert_with(Vec::new).push(value);
    }
    Ok(fps)
}

/// Reconstructed pilot measurements. Fewer than three runs on any included arm are not a pilot.
pub fn pilot_fps(
    campaign: &File,
    uid: u32,
    pilot: &ChildStudy,
) -> anyhow::Result<BTreeMap<ProtonSource, Vec<f64>>> {
    let studies = File::from(crate::syscall::openat_dir_nofollow(
        campaign.as_fd(),
        "studies",
    )?);
    pilot_fps_in_studies(&studies, uid, pilot)
}

pub fn pilot_fps_in_studies(
    studies: &File,
    uid: u32,
    pilot: &ChildStudy,
) -> anyhow::Result<BTreeMap<ProtonSource, Vec<f64>>> {
    let mut fps = screen_fps_in_studies(studies, uid, pilot)?;
    anyhow::ensure!(
        fps.values()
            .all(|runs| runs.len() <= super::pilot_rule::MIN_PILOT_RUNS),
        "PilotInvalid"
    );
    fps.retain(|_, runs| runs.len() == super::pilot_rule::MIN_PILOT_RUNS);
    anyhow::ensure!(
        fps.len() >= 2
            && fps
                .values()
                .all(|runs| runs.len() >= super::pilot_rule::MIN_PILOT_RUNS),
        "PilotTooThin"
    );
    Ok(fps)
}

#[cfg(test)]
#[path = "arm_selection_tests.rs"]
mod tests;
