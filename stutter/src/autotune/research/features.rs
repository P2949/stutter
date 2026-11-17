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

/// Merge the environments of the study's factors for one treatment (`on` is the study arm, off the
/// control arm): the catalog's own value for each, so the control arm carries the explicit baseline
/// (for example `PROTON_NO_NTSYNC=1` for NTSync) rather than silently nothing. Every other factor
/// stays at baseline by absence. Conflicting values for one key are refused, not resolved silently.
pub fn arm_env(
    source: ProtonSource,
    enabled: &[ResearchFactor],
    on: bool,
) -> Result<Env, (ResearchFactor, FactorBlock)> {
    let mut merged = Env::new();
    for factor in enabled {
        for (key, value) in factor_env(source, *factor, on).map_err(|block| (*factor, block))? {
            if merged.get(&key).is_some_and(|existing| *existing != value) {
                return Err((*factor, FactorBlock::ConflictingEnvironment));
            }
            merged.insert(key, value);
        }
    }
    debug_assert!(merged.keys().all(|key| controlled(key)));
    Ok(merged)
}

/// Whether a launched game process's environment (`/proc/<pid>/environ`, NUL-separated) shows this
/// treatment really reached it: every variable the treatment sets is present with its value, and no
/// variable only the other treatment sets is. This is activation of the environment control, not
/// proof that the feature works. A factor with no environment difference between its treatments
/// cannot be shown this way, and is never reported as reached.
pub fn treatment_effective(
    environ: &[u8],
    source: ProtonSource,
    factor: ResearchFactor,
    on: bool,
) -> bool {
    let (Ok(wanted), Ok(other)) = (
        factor_env(source, factor, on),
        factor_env(source, factor, !on),
    ) else {
        return false;
    };
    if wanted == other {
        return false;
    }
    let present: BTreeMap<String, String> = environ
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let text = String::from_utf8_lossy(entry);
            let (key, value) = text.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect();
    wanted
        .iter()
        .all(|(key, value)| present.get(key) == Some(value))
        && other
            .iter()
            .filter(|(key, value)| wanted.get(*key) != Some(*value))
            .all(|(key, value)| present.get(key) != Some(value))
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

/// One (source, factor) cell of the G7 feature matrix. A planned cell still needs runtime
/// activation proof and a functional check before any performance run; it never implies a gain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatrixCell {
    Planned { on_env: Env, off_env: Env },
    Blocked(FactorBlock),
}

pub const ALL_FACTORS: [ResearchFactor; 10] = {
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

pub const ALL_SOURCES: [ProtonSource; 6] = [
    ProtonSource::ValveStable,
    ProtonSource::ValveExperimental,
    ProtonSource::CachyOs,
    ProtonSource::CachyOsV3,
    ProtonSource::Ge,
    ProtonSource::Em,
];

/// Every source x factor cell, so missing coverage is a typed block rather than an omission.
pub fn feature_matrix() -> Vec<(ProtonSource, ResearchFactor, MatrixCell)> {
    let mut cells = Vec::with_capacity(ALL_SOURCES.len() * ALL_FACTORS.len());
    for source in ALL_SOURCES {
        for factor in ALL_FACTORS {
            let cell = match (
                factor_env(source, factor, true),
                factor_env(source, factor, false),
            ) {
                (Ok(on_env), Ok(off_env)) => MatrixCell::Planned { on_env, off_env },
                (Err(block), _) | (_, Err(block)) => MatrixCell::Blocked(block),
            };
            cells.push((source, factor, cell));
        }
    }
    cells
}

/// What can honestly be said of a (source, factor) cell today. Nothing here is `FunctionallyProven`: that
/// needs a factor-specific functional receipt (a measured effect of the feature itself, not an
/// environment variable reaching the game), and none exists for any factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum FeatureDisposition {
    /// The factor does not apply to this build or is not a Proton environment factor at all.
    NotApplicable,
    /// The build documents no (or conflicting) environment control for it: it cannot be switched.
    Unavailable,
    /// Whether it took effect depends on output, compositor and driver that must qualify first.
    NotObservable,
    /// An environment control exists and its reaching the game can be observed, but no functional
    /// receipt shows the feature works.
    ControlAvailableNotFunctionallyProven,
    /// A functional receipt shows the feature worked (unreachable until such receipts exist).
    FunctionallyProven,
}

/// Every (source, factor) cell with its disposition, for reports.
pub fn feature_dispositions() -> Vec<(ProtonSource, ResearchFactor, FeatureDisposition)> {
    feature_matrix()
        .into_iter()
        .map(|(source, factor, cell)| {
            let disposition = match cell {
                MatrixCell::Planned { .. } => {
                    FeatureDisposition::ControlAvailableNotFunctionallyProven
                }
                MatrixCell::Blocked(
                    FactorBlock::NotApplicableToSource | FactorBlock::NotAProtonEnvironmentFactor,
                ) => FeatureDisposition::NotApplicable,
                MatrixCell::Blocked(
                    FactorBlock::NoEnvironmentControl | FactorBlock::ConflictingEnvironment,
                ) => FeatureDisposition::Unavailable,
                MatrixCell::Blocked(FactorBlock::NeedsQualifiedOutput) => {
                    FeatureDisposition::NotObservable
                }
            };
            (source, factor, disposition)
        })
        .collect()
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
            true,
        )
        .unwrap();
        assert_eq!(on["PROTON_ENABLE_WAYLAND"], "1");
        assert_eq!(on["MLFG_UPGRADE"], "1");
        assert!(arm_env(ProtonSource::Ge, &[ResearchFactor::AntiLag2], true).is_err());
        assert_eq!(
            factor_env(ProtonSource::Ge, ResearchFactor::NtSync, false).unwrap()["PROTON_NO_NTSYNC"],
            "1"
        );
    }

    #[test]
    fn each_treatment_runs_the_catalogs_own_environment_and_ntsync_arms_differ() {
        for source in SOURCES {
            for factor in ALL {
                for on in [true, false] {
                    if let Ok(expected) = factor_env(source, factor, on) {
                        assert_eq!(
                            arm_env(source, &[factor], on).unwrap(),
                            expected,
                            "{source:?} {factor:?} on={on}"
                        );
                    }
                }
            }
        }
        // The control arm of NTSync disables it; an empty control would equal the study arm.
        for source in [ProtonSource::Ge, ProtonSource::CachyOs, ProtonSource::Em] {
            let on = arm_env(source, &[ResearchFactor::NtSync], true).unwrap();
            let off = arm_env(source, &[ResearchFactor::NtSync], false).unwrap();
            assert_ne!(on, off, "{source:?}");
            assert_eq!(off["PROTON_NO_NTSYNC"], "1");
        }
        // The Wayland control is explicit, not absent.
        let off = arm_env(ProtonSource::Ge, &[ResearchFactor::WineWayland], false).unwrap();
        assert_eq!(off["PROTON_ENABLE_WAYLAND"], "0");
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

#[cfg(test)]
mod matrix_tests {
    use super::*;

    #[test]
    fn matrix_covers_every_cell_and_plans_only_controlled_env() {
        let cells = feature_matrix();
        assert_eq!(cells.len(), 60);
        for (_, _, cell) in &cells {
            if let MatrixCell::Planned { on_env, off_env } = cell {
                assert!(on_env.keys().chain(off_env.keys()).all(|k| controlled(k)));
            }
        }
    }

    #[test]
    fn hdr_is_never_a_planned_environment_cell() {
        assert!(
            feature_matrix()
                .iter()
                .filter(|(_, f, _)| *f == ResearchFactor::Hdr)
                .all(|(_, _, c)| matches!(c, MatrixCell::Blocked(_)))
        );
    }
}

#[cfg(test)]
mod effective_tests {
    use super::*;

    fn environ(pairs: &[(&str, &str)]) -> Vec<u8> {
        pairs
            .iter()
            .flat_map(|(key, value)| format!("{key}={value}\0").into_bytes())
            .collect()
    }

    #[test]
    fn a_treatment_is_reached_only_when_its_own_variables_show_and_the_others_do_not() {
        use ProtonSource::Ge;
        use ResearchFactor::{NtSync, WineWayland};
        // Wayland on needs PROTON_ENABLE_WAYLAND=1 (and not the off value 0).
        assert!(treatment_effective(
            &environ(&[("PROTON_ENABLE_WAYLAND", "1")]),
            Ge,
            WineWayland,
            true
        ));
        assert!(!treatment_effective(
            &environ(&[("PROTON_ENABLE_WAYLAND", "0")]),
            Ge,
            WineWayland,
            true
        ));
        assert!(!treatment_effective(&environ(&[]), Ge, WineWayland, true));
        // Off sets the documented baseline value explicitly.
        assert!(treatment_effective(
            &environ(&[("PROTON_ENABLE_WAYLAND", "0")]),
            Ge,
            WineWayland,
            false
        ));
        assert!(!treatment_effective(
            &environ(&[("PROTON_ENABLE_WAYLAND", "1")]),
            Ge,
            WineWayland,
            false
        ));
        // NTSync on is the absence of the disabling variable; off is its presence.
        assert!(treatment_effective(
            &environ(&[("HOME", "/h")]),
            Ge,
            NtSync,
            true
        ));
        assert!(!treatment_effective(
            &environ(&[("PROTON_NO_NTSYNC", "1")]),
            Ge,
            NtSync,
            true
        ));
        assert!(treatment_effective(
            &environ(&[("PROTON_NO_NTSYNC", "1")]),
            Ge,
            NtSync,
            false
        ));
        assert!(!treatment_effective(
            &environ(&[("HOME", "/h")]),
            Ge,
            NtSync,
            false
        ));
    }

    #[test]
    fn a_build_without_a_control_or_a_factor_with_no_environment_difference_is_never_reached() {
        use ProtonSource::ValveStable;
        use ResearchFactor::{Gamescope, NtSync};
        assert!(!treatment_effective(
            &environ(&[("PROTON_NO_NTSYNC", "1")]),
            ValveStable,
            NtSync,
            false
        ));
        assert!(!treatment_effective(
            &environ(&[]),
            ProtonSource::Ge,
            Gamescope,
            true
        ));
    }
}

#[cfg(test)]
mod disposition_tests {
    use super::*;

    fn of(source: ProtonSource, factor: ResearchFactor) -> FeatureDisposition {
        feature_dispositions()
            .into_iter()
            .find(|(s, f, _)| *s == source && *f == factor)
            .map(|(_, _, disposition)| disposition)
            .unwrap()
    }

    #[test]
    fn every_cell_has_a_disposition_and_none_is_functionally_proven() {
        let all = feature_dispositions();
        assert_eq!(all.len(), 60);
        assert!(
            all.iter()
                .all(|(_, _, d)| *d != FeatureDisposition::FunctionallyProven),
            "no factor has a functional receipt"
        );
    }

    #[test]
    fn unavailable_not_applicable_and_unproven_are_different_things() {
        use ProtonSource::{CachyOs, Em, Ge, ValveStable};
        use ResearchFactor::{AntiLag2, Fsr4, Gamescope, NtSync};
        // A control exists, so it can be switched and its reaching the game observed; it is not proven.
        assert_eq!(
            of(Ge, NtSync),
            FeatureDisposition::ControlAvailableNotFunctionallyProven
        );
        // Documented but without an environment control: cannot be switched here.
        assert_eq!(of(CachyOs, Fsr4), FeatureDisposition::Unavailable);
        assert_eq!(of(Em, AntiLag2), FeatureDisposition::Unavailable);
        // Not a Proton environment factor at all, or not for this build.
        assert_eq!(of(Ge, Gamescope), FeatureDisposition::NotApplicable);
        assert_eq!(of(ValveStable, NtSync), FeatureDisposition::NotApplicable);
    }
}
