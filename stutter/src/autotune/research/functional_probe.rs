//! Functional probes for feature studies (PLAN G7). Activation says a factor's environment variable
//! reached the game process; it does not say the feature worked. Where the game itself reports the
//! feature's state in its sealed native result, that report is a functional observation: frame
//! generation appears as `frameGenerationType` and the upscaler as `upscalingType` in the native
//! context. A nonzero enum identifies only a game-reported selection; it does not identify FSR4,
//! GE's replacement DLL, native frame generation or EM's MLFG, nor prove they function. Every other
//! factor has no such report, so it is `NotObservable` here.

use super::{ResearchFactor, Treatment, native_context::NativeContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Functional {
    /// On attempts reported a nonzero selection and Off attempts reported zero. This alone is not
    /// proof of a particular technology, loaded backend, replacement DLL, or effective operation.
    GameReportedSelection,
    /// The game reported the feature's state and it contradicts the treatment (an On attempt without
    /// the feature, or an Off attempt with it).
    NotObserved,
    /// The game's own result carries no report for this factor.
    NotObservable,
    /// A treatment had no attempt, or a context lacked the field.
    Unproven,
}

/// The game-reported value that says the factor is in use, from one result's context.
fn reported(factor: ResearchFactor, context: &NativeContext) -> Option<Option<i64>> {
    match factor {
        ResearchFactor::FrameGeneration => Some(context.frame_generation_type),
        ResearchFactor::Fsr4 => Some(context.upscaling_type),
        _ => None,
    }
}

/// Judge a feature study's completed attempts, each as its treatment and sealed context.
pub fn probe(
    factor: ResearchFactor,
    attempts: &[(Treatment, Option<&NativeContext>)],
) -> Functional {
    if !matches!(
        factor,
        ResearchFactor::FrameGeneration | ResearchFactor::Fsr4
    ) {
        return Functional::NotObservable;
    }
    let (mut on, mut off) = (0usize, 0usize);
    for (treatment, context) in attempts {
        let Some(Some(value)) = context.and_then(|context| reported(factor, context)) else {
            return Functional::Unproven;
        };
        let in_use = value != 0;
        match (treatment, in_use) {
            (Treatment::On, true) => on += 1,
            (Treatment::Off, false) => off += 1,
            _ => return Functional::NotObserved,
        }
    }
    if on > 0 && off > 0 {
        Functional::GameReportedSelection
    } else {
        Functional::Unproven
    }
}

#[cfg(test)]
#[path = "functional_probe_tests.rs"]
mod tests;
