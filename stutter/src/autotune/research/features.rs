//! Typed Proton feature catalog derived from the NixosConf `features.json` facts. A factor only
//! ever becomes an environment the dispatcher will apply when the selected fork documents it; the
//! rest are typed blocks, never silent no-ops. Activation and function are proven separately from
//! performance, and a flag without runtime activation is `NotActivated`.

use std::collections::BTreeMap;

use super::{ProtonSource, ResearchFactor, dispatcher::controlled};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactorBlock {
    NotApplicableToSource,
    /// The fork documents the feature but exposes no environment control for it.
    NoEnvironmentControl,
    /// Not a Proton environment factor (for example a separate compositor wrapper).
    NotAProtonEnvironmentFactor,
    /// Output, compositor and driver must independently qualify before any claim.
    NeedsQualifiedOutput,
    ConflictingEnvironment,
}

type Env = BTreeMap<String, String>;

fn env(pairs: &[(&str, &str)]) -> Env {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn custom(source: ProtonSource) -> bool {
    !matches!(
        source,
        ProtonSource::ValveStable | ProtonSource::ValveExperimental
    )
}

/// Environment for one factor in the study's "on" or "off" arm. "Off" is the fork's documented
/// baseline, set explicitly so an ambient value can never leak into the control arm.
pub fn factor_env(
    source: ProtonSource,
    factor: ResearchFactor,
    on: bool,
) -> Result<Env, FactorBlock> {
    use ProtonSource::{CachyOs, CachyOsV3, Em, Ge};
    use ResearchFactor::*;
    let cachy = matches!(source, CachyOs | CachyOsV3);
    match factor {
        WineWayland if custom(source) => Ok(env(&[(
            "PROTON_ENABLE_WAYLAND",
            if on { "1" } else { "0" },
        )])),
        Vkd3dLowLatency if cachy => Ok(env(&[(
            "PROTON_VKD3D_LOWLATENCY",
            if on { "1" } else { "0" },
        )])),
        // NTSync is the baseline everywhere custom; the study arm is the disabled one.
        NtSync if custom(source) => Ok(if on {
            Env::new()
        } else {
            env(&[("PROTON_NO_NTSYNC", "1")])
        }),
        Fsr4 if source == Ge => Ok(if on {
            env(&[("PROTON_FSR4_UPGRADE", "4.1.1")])
        } else {
            Env::new()
        }),
        Fsr4 if cachy || source == Em => Err(FactorBlock::NoEnvironmentControl),
        FrameGeneration if source == Em => Ok(if on {
            env(&[("MLFG_UPGRADE", "1")])
        } else {
            Env::new()
        }),
        FrameGeneration if cachy => Err(FactorBlock::NoEnvironmentControl),
        AntiLag2 if source == Em => Err(FactorBlock::NoEnvironmentControl),
        Gamescope => Err(FactorBlock::NotAProtonEnvironmentFactor),
        AudioBackend if cachy => Ok(if on {
            env(&[("PROTON_USE_PIPEWIRE", "0")])
        } else {
            Env::new()
        }),
        InputBackend if source == Em => Ok(if on {
            env(&[("PROTON_USE_SDL", "1")])
        } else {
            Env::new()
        }),
        Hdr if custom(source) => Err(FactorBlock::NeedsQualifiedOutput),
        _ => Err(FactorBlock::NotApplicableToSource),
    }
}

/// Merge the "on" environments for the enabled factors; every other factor stays at baseline by
/// absence. Conflicting values for one key are refused rather than resolved silently.
pub fn arm_env(
    source: ProtonSource,
    enabled: &[ResearchFactor],
) -> Result<Env, (ResearchFactor, FactorBlock)> {
    let mut merged = Env::new();
    for factor in enabled {
        for (key, value) in factor_env(source, *factor, true).map_err(|block| (*factor, block))? {
            if merged.get(&key).is_some_and(|existing| *existing != value) {
                return Err((*factor, FactorBlock::ConflictingEnvironment));
            }
            merged.insert(key, value);
        }
    }
    debug_assert!(merged.keys().all(|key| controlled(key)));
    Ok(merged)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureEvidence {
    /// A runtime proof (DLL, backend or effective-path), not merely that the flag was set.
    pub activated: bool,
    pub functional: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureVerdict {
    NotActivated,
    ActivatedNonFunctional,
    /// Eligible for a repeated performance comparison; says nothing about the gain.
    ActivatedFunctional,
}

pub fn classify_feature(evidence: FeatureEvidence) -> FeatureVerdict {
    match (evidence.activated, evidence.functional) {
        (false, _) => FeatureVerdict::NotActivated,
        (true, false) => FeatureVerdict::ActivatedNonFunctional,
        (true, true) => FeatureVerdict::ActivatedFunctional,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ResearchFactor; 10] = {
        use ResearchFactor::*;
        [
            WineWayland,
            Vkd3dLowLatency,
            NtSync,
            Fsr4,
            FrameGeneration,
            AntiLag2,
            Gamescope,
            AudioBackend,
            InputBackend,
            Hdr,
        ]
    };
    const SOURCES: [ProtonSource; 6] = [
        ProtonSource::ValveStable,
        ProtonSource::ValveExperimental,
        ProtonSource::CachyOs,
        ProtonSource::CachyOsV3,
        ProtonSource::Ge,
        ProtonSource::Em,
    ];

    #[test]
    fn every_documented_environment_is_accepted_by_the_dispatcher_and_nothing_else_is() {
        for source in SOURCES {
            for factor in ALL {
                for on in [true, false] {
                    if let Ok(env) = factor_env(source, factor, on) {
                        assert!(
                            env.keys().all(|key| controlled(key)),
                            "{source:?} {factor:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn valve_builds_have_no_custom_features_and_unsupported_pairs_are_typed_blocks() {
        for factor in ALL {
            assert!(factor_env(ProtonSource::ValveStable, factor, true).is_err());
        }
        assert_eq!(
            factor_env(ProtonSource::Em, ResearchFactor::Vkd3dLowLatency, true),
            Err(FactorBlock::NotApplicableToSource)
        );
        assert_eq!(
            factor_env(ProtonSource::Ge, ResearchFactor::Hdr, true),
            Err(FactorBlock::NeedsQualifiedOutput)
        );
        assert_eq!(
            factor_env(ProtonSource::Em, ResearchFactor::AntiLag2, true),
            Err(FactorBlock::NoEnvironmentControl)
        );
        assert_eq!(
            factor_env(ProtonSource::Ge, ResearchFactor::Gamescope, true),
            Err(FactorBlock::NotAProtonEnvironmentFactor)
        );
    }

    #[test]
    fn baseline_is_explicit_and_arms_merge_without_silent_conflict() {
        let off = factor_env(ProtonSource::Ge, ResearchFactor::WineWayland, false).unwrap();
        assert_eq!(off["PROTON_ENABLE_WAYLAND"], "0");
        let on = arm_env(
            ProtonSource::Em,
            &[ResearchFactor::WineWayland, ResearchFactor::FrameGeneration],
        )
        .unwrap();
        assert_eq!(on["PROTON_ENABLE_WAYLAND"], "1");
        assert_eq!(on["MLFG_UPGRADE"], "1");
        assert!(arm_env(ProtonSource::Ge, &[ResearchFactor::AntiLag2]).is_err());
        assert_eq!(
            factor_env(ProtonSource::Ge, ResearchFactor::NtSync, false).unwrap()["PROTON_NO_NTSYNC"],
            "1"
        );
    }

    #[test]
    fn a_flag_without_runtime_proof_is_not_activated() {
        let verdict = |activated, functional| {
            classify_feature(FeatureEvidence {
                activated,
                functional,
            })
        };
        assert_eq!(verdict(false, true), FeatureVerdict::NotActivated);
        assert_eq!(verdict(true, false), FeatureVerdict::ActivatedNonFunctional);
        assert_eq!(verdict(true, true), FeatureVerdict::ActivatedFunctional);
    }
}
