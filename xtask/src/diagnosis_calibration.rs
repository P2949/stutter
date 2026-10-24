//! Per-family diagnosis threshold calibration over a frozen corpus. The sweep drives the
//! production report pipeline (`family_calibration_outcome`); this module only labels,
//! scores and decides. It never edits thresholds: a change is a reviewed code change backed
//! by the retained result, and without enough evidence the decision is to retain.
use std::{fs, path::Path};

use anyhow::{Context, bail, ensure};
use serde_json::json;
use stutter::api::report::{CalibrationFamily, family_calibration_outcome};

/// Evidence floors below which the decision is always "retain".
const MIN_REAL_POSITIVES: usize = 10;
const MIN_REAL_NEGATIVES: usize = 10;
const MIN_MIXED: usize = 3;

pub struct FamilySpec {
    pub name: &'static str,
    pub family: CalibrationFamily,
    /// Expected primary causes that make a fixture a positive for this family.
    pub positives: &'static [&'static str],
    pub unit: &'static str,
    /// Multiplier from the displayed unit to the library value (ns or percent).
    pub scale: f64,
    pub current: f64,
    pub sweep: &'static [f64],
}

pub const FAMILIES: [FamilySpec; 4] = [
    FamilySpec {
        name: "scheduler",
        family: CalibrationFamily::Scheduler,
        positives: &["CompositorSchedulerDelay", "GameThreadSchedulerDelay"],
        unit: "ms",
        scale: 1_000_000.0,
        current: 2.0,
        sweep: &[0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0],
    },
    FamilySpec {
        name: "io",
        family: CalibrationFamily::BlockIo,
        positives: &["BlockIoCandidate"],
        unit: "ms",
        scale: 1_000_000.0,
        current: 1.0,
        sweep: &[0.25, 0.5, 1.0, 2.0, 4.0],
    },
    FamilySpec {
        name: "irq",
        family: CalibrationFamily::Irq,
        positives: &["IrqDelayCandidate"],
        unit: "ms",
        scale: 1_000_000.0,
        current: 0.25,
        sweep: &[0.05, 0.1, 0.25, 0.5, 1.0],
    },
    FamilySpec {
        name: "gpu",
        family: CalibrationFamily::Gpu,
        positives: &["GpuBoundCandidate"],
        unit: "percent busy",
        scale: 1.0,
        current: 95.0,
        sweep: &[80.0, 85.0, 90.0, 95.0, 98.0],
    },
];

pub fn family_spec(name: &str) -> anyhow::Result<&'static FamilySpec> {
    FAMILIES
        .iter()
        .find(|spec| spec.name == name)
        .with_context(|| format!("unknown family {name}; expected one of scheduler, io, irq, gpu"))
}

mod corpus;
mod sweep;
pub use corpus::*;
use sweep::{mixed_sweep, partition, sweep};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counts {
    pub tp: usize,
    pub fp: usize,
    pub fn_: usize,
    pub tn: usize,
    /// Negative controls called a family positive with High observation confidence.
    pub false_high_observation: usize,
    /// Negative controls called a family positive with High *causal* confidence.
    pub false_high_causal: usize,
}
impl Counts {
    pub fn precision(&self) -> Option<f64> {
        (self.tp + self.fp > 0).then(|| self.tp as f64 / (self.tp + self.fp) as f64)
    }
    pub fn recall(&self) -> Option<f64> {
        (self.tp + self.fn_ > 0).then(|| self.tp as f64 / (self.tp + self.fn_) as f64)
    }
}

/// The primary cause the pipeline produced with its two confidence dimensions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prediction {
    pub cause: String,
    pub observation: String,
    pub causal: String,
}

/// Cause scoring depends only on the predicted cause; the confidence dimensions are tallied
/// separately so threshold detection stays independent of causal-confidence scoring.
pub fn score(spec: &FamilySpec, labels: &[(bool, Option<Prediction>)]) -> Counts {
    let mut counts = Counts::default();
    for (positive, predicted) in labels {
        let hit = predicted
            .as_ref()
            .is_some_and(|p| spec.positives.contains(&p.cause.as_str()));
        match (positive, hit) {
            (true, true) => counts.tp += 1,
            (true, false) => counts.fn_ += 1,
            (false, true) => {
                counts.fp += 1;
                if predicted.as_ref().is_some_and(|p| p.observation == "High") {
                    counts.false_high_observation += 1;
                }
                if predicted.as_ref().is_some_and(|p| p.causal == "High") {
                    counts.false_high_causal += 1;
                }
            }
            (false, false) => counts.tn += 1,
        }
    }
    counts
}

pub fn run(
    root: &Path,
    corpus: &Path,
    out: Option<&Path>,
    family_name: &str,
) -> anyhow::Result<()> {
    let report = report(root, corpus, family_name)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(out) = out {
        fs::write(root.join(out), serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}

/// The calibration report of one family over the corpus at `root/corpus`.
pub fn report(root: &Path, corpus: &Path, family_name: &str) -> anyhow::Result<serde_json::Value> {
    let spec = family_spec(family_name)?;
    let (fixtures, frozen_id) = load_corpus(&root.join(corpus))?;
    let diagnose = |fixture: &Fixture, value: f64| {
        family_calibration_outcome(&fixture.dir, spec.family, value * spec.scale)
            .with_context(|| format!("diagnose {}", fixture.name))
    };
    // Decision data: explicit positives and matched negative controls only; mixed captures
    // have their own section and legacy fixtures a separate regression-only view.
    let (decision, duplicates) = decision_set(spec, &fixtures);
    let (binary, mixed) = partition(&decision);
    let decision_sweep = sweep(spec, &binary, &diagnose)?;
    let mixed_results = mixed_sweep(spec, &mixed, &diagnose)?;
    let legacy_items = fixtures
        .iter()
        .filter(|f| f.meta.is_none())
        .filter_map(|f| f.label(spec).map(|positive| (f, positive)))
        .collect::<Vec<_>>();
    ensure!(
        !legacy_items.is_empty() || !decision.is_empty(),
        "corpus has no labeled fixtures"
    );
    let regression_sweep = sweep(spec, &legacy_items, &diagnose)?;
    let floors = family_floors(spec, &fixtures);
    let decision_name = if floors.sufficient() {
        "evidence_sufficient_for_review"
    } else {
        "retain_current_threshold_evidence_insufficient"
    };
    let report = json!({"schema_version":3,"family":spec.name,"corpus_id":frozen_id,"current_threshold":spec.current,"unit":spec.unit,
        "corpus":{"decision_positives":floors.positives,"decision_negative_controls":floors.negative_controls,
            "decision_mixed_confounded":floors.mixed_confounded,"regression_only_legacy":legacy_items.len(),
            "duplicate_capture_ids":duplicates},
        "sufficiency":{"positives":floors.positives,"negative_controls":floors.negative_controls,
            "mixed_confounded":floors.mixed_confounded,"ineligible_real_captures":floors.ineligible_real,
            "duplicate_capture_ids":floors.duplicate_capture_ids,"sufficient":floors.sufficient()},
        "evidence_floors":{"positive":MIN_REAL_POSITIVES,"negative_control":MIN_REAL_NEGATIVES,"mixed_confounded":MIN_MIXED},
        "sweep":decision_sweep,"mixed_sweep":mixed_results,"regression_sweep":regression_sweep,"decision":decision_name,
        "limitations":["family-level labels; specific-cause agreement is not scored",
            "the decision sweep uses only explicit positives and matched negative controls for this family; mixed captures are reported separately and legacy inferred fixtures only in the regression-only sweep",
            "sufficiency counts only captures with an explicit non-synthetic evidence_kind, complete provenance, an explicit control_type and [coverage].diagnoses naming the family",
            "other families' positives are never this family's negative controls; a mixed capture counts only for the families it names",
            "observation and causal confidence are reported separately; causal confidence is capped by the explicit-attribution-chain policy and visible-stutter linkage, and no causal threshold is tuned here",
            "overlap is not proven causality: IRQ/I-O overlap fixtures do not calibrate causal claims",
            "numerical optimum is never chosen mechanically; false high-confidence claims weigh more than inconclusive ones"]});
    if !FAMILIES.iter().any(|f| f.name == family_name) {
        bail!("unreachable family");
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring_counts_false_high_observation_and_causal_separately() {
        let spec = family_spec("scheduler").unwrap();
        let hit = |observation: &str, causal: &str| {
            Some(Prediction {
                cause: "GameThreadSchedulerDelay".to_owned(),
                observation: observation.to_owned(),
                causal: causal.to_owned(),
            })
        };
        let counts = score(
            spec,
            &[
                (true, hit("High", "Medium")),
                (true, None),
                // High observation, but overlap alone never established causality.
                (false, hit("High", "NotEstablished")),
                (false, hit("Low", "Low")),
                (false, hit("High", "High")),
                (
                    false,
                    Some(Prediction {
                        cause: "IrqDelayCandidate".into(),
                        observation: "High".into(),
                        causal: "High".into(),
                    }),
                ),
            ],
        );
        assert_eq!((counts.tp, counts.fn_, counts.fp, counts.tn), (1, 1, 3, 1));
        assert_eq!(counts.false_high_observation, 2);
        assert_eq!(counts.false_high_causal, 1);
        assert_eq!(counts.precision(), Some(0.25));
        assert_eq!(counts.recall(), Some(0.5));
        assert_eq!(Counts::default().precision(), None);
    }

    #[test]
    fn every_family_current_value_is_in_its_sweep_and_matches_the_defaults() {
        for spec in &FAMILIES {
            assert!(spec.sweep.contains(&spec.current), "{}", spec.name);
        }
        // Defaults reproduce the corpus expectations through the production pipeline.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let (fixtures, id) = load_corpus(&root.join("stutter/tests/fixtures/runs")).unwrap();
        assert_eq!(id.len(), 64);
        for (family, fixture, cause) in [
            (
                "scheduler",
                "game_thread_scheduler_delay",
                "GameThreadSchedulerDelay",
            ),
            ("io", "block_io_stall", "BlockIoCandidate"),
            ("irq", "irq_heavy", "IrqDelayCandidate"),
            ("gpu", "gpu_bound_clean_cpu", "GpuBoundCandidate"),
        ] {
            let spec = family_spec(family).unwrap();
            let fixture = fixtures.iter().find(|f| f.name == fixture).unwrap();
            let outcome =
                family_calibration_outcome(&fixture.dir, spec.family, spec.current * spec.scale)
                    .unwrap()
                    .unwrap();
            assert_eq!(outcome.cause, cause, "{family}");
        }
    }
}
