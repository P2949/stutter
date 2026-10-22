//! Per-family diagnosis threshold calibration over a frozen corpus. The sweep drives the
//! production report pipeline (`family_calibration_outcome`); this module only labels,
//! scores and decides. It never edits thresholds: a change is a reviewed code change backed
//! by the retained result, and without enough evidence the decision is to retain.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use serde_json::json;
use stutter::api::report::{CalibrationFamily, family_calibration_outcome};

use crate::{
    evidence_bundle::sha256::sha256_hex,
    fixture_coverage::{CalibrationMeta, parse_fixture_toml_text},
};

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Fixture {
    pub name: String,
    pub dir: PathBuf,
    /// Name/source inference of "real". Regression sweeps may use it; sufficiency never does.
    pub legacy_real: bool,
    pub expected_cause: String,
    /// Explicit calibration metadata when the capture is eligible for an evidence floor.
    pub meta: Option<CalibrationMeta>,
    /// Why `meta` is absent.
    pub ineligible_reason: String,
}

impl Fixture {
    /// `Some(true)` positive for the family, `Some(false)` negative control, `None` unlabeled.
    pub fn label(&self, spec: &FamilySpec) -> Option<bool> {
        match self.expected_cause.as_str() {
            "" | "Any" => None,
            cause => Some(spec.positives.contains(&cause)),
        }
    }

    /// Whether the capture explicitly names this family in `[coverage].diagnoses`.
    fn names_family(meta: &CalibrationMeta, spec: &FamilySpec) -> bool {
        meta.diagnoses
            .iter()
            .any(|diagnosis| spec.positives.contains(&diagnosis.as_str()))
    }
}

/// Eligible captures per family and role; only these can satisfy the evidence floors.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Floors {
    pub positives: usize,
    pub negative_controls: usize,
    pub mixed_confounded: usize,
    /// Real-looking captures that cannot count (legacy inference, synthetic, incomplete).
    pub ineligible_real: usize,
    /// Eligible captures sharing a capture id with an earlier one (counted once).
    pub duplicate_capture_ids: usize,
}

impl Floors {
    pub fn sufficient(&self) -> bool {
        self.positives >= MIN_REAL_POSITIVES
            && self.negative_controls >= MIN_REAL_NEGATIVES
            && self.mixed_confounded >= MIN_MIXED
    }
}

/// Counts evidence floors for one family. A capture counts only when its explicit evidence
/// kind is non-synthetic, its provenance is complete, its `control_type` is explicit and its
/// `[coverage].diagnoses` names the family; other families' positives are not negative
/// controls, and a mixed capture counts only for the families it names.
pub fn family_floors(spec: &FamilySpec, fixtures: &[Fixture]) -> Floors {
    let mut floors = Floors::default();
    let mut seen = std::collections::BTreeSet::new();
    for fixture in fixtures {
        let Some(meta) = &fixture.meta else {
            floors.ineligible_real += usize::from(fixture.legacy_real);
            continue;
        };
        if !Fixture::names_family(meta, spec) {
            continue;
        }
        if !seen.insert(meta.capture_id.clone()) {
            floors.duplicate_capture_ids += 1;
            continue;
        }
        match meta.control_type.as_str() {
            "positive" => floors.positives += 1,
            "negative_control" => floors.negative_controls += 1,
            "mixed_confounded" => floors.mixed_confounded += 1,
            _ => {}
        }
    }
    floors
}

pub fn parse_fixture(dir: &Path, text: &str) -> Fixture {
    let summary = parse_fixture_toml_text(text);
    let (meta, ineligible_reason) = match summary.calibration_meta() {
        Ok(meta) => (Some(meta), String::new()),
        Err(reason) => (None, reason),
    };
    Fixture {
        name: summary.name().to_owned(),
        dir: dir.to_owned(),
        legacy_real: summary.is_legacy_real(),
        expected_cause: summary.primary_cause().to_owned(),
        meta,
        ineligible_reason,
    }
}

pub fn load_corpus(corpus: &Path) -> anyhow::Result<(Vec<Fixture>, String)> {
    let mut dirs = fs::read_dir(corpus)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("fixture.toml").is_file())
        .collect::<Vec<_>>();
    dirs.sort();
    let mut fixtures = Vec::new();
    let mut frozen = String::new();
    for dir in dirs {
        let text = fs::read_to_string(dir.join("fixture.toml"))?;
        frozen.push_str(&format!(
            "{}\n{text}\n",
            dir.file_name().and_then(|n| n.to_str()).unwrap_or("")
        ));
        fixtures.push(parse_fixture(&dir, &text));
    }
    Ok((fixtures, sha256_hex(frozen.as_bytes())))
}

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
    let spec = family_spec(family_name)?;
    let (fixtures, frozen_id) = load_corpus(&root.join(corpus))?;
    let labeled = fixtures
        .iter()
        .filter(|f| f.label(spec).is_some())
        .collect::<Vec<_>>();
    ensure!(!labeled.is_empty(), "corpus has no labeled fixtures");
    let diagnose = |fixture: &Fixture, value: f64| {
        family_calibration_outcome(&fixture.dir, spec.family, value * spec.scale)
            .with_context(|| format!("diagnose {}", fixture.name))
    };
    // Agreement is measured against the current threshold, computed up front so sweep
    // order cannot matter.
    let mut baseline: BTreeMap<String, String> = BTreeMap::new();
    for fixture in &labeled {
        baseline.insert(
            fixture.name.clone(),
            diagnose(fixture, spec.current)?
                .map(|o| o.cause)
                .unwrap_or_default(),
        );
    }
    let mut results = Vec::new();
    for value in spec.sweep {
        let (mut labels, mut stable) = (Vec::new(), 0);
        let mut observation: BTreeMap<String, usize> = BTreeMap::new();
        let mut causal: BTreeMap<String, usize> = BTreeMap::new();
        for fixture in &labeled {
            let outcome = diagnose(fixture, *value)?;
            let cause = outcome
                .as_ref()
                .map(|o| o.cause.clone())
                .unwrap_or_default();
            if baseline.get(&fixture.name).is_some_and(|b| *b == cause) {
                stable += 1;
            }
            if let Some(o) = outcome
                .as_ref()
                .filter(|_| spec.positives.contains(&cause.as_str()))
            {
                *observation.entry(o.confidence.clone()).or_default() += 1;
                *causal.entry(o.causal_confidence.clone()).or_default() += 1;
            }
            labels.push((
                fixture.label(spec) == Some(true),
                outcome.map(|o| Prediction {
                    cause: o.cause,
                    observation: o.confidence,
                    causal: o.causal_confidence,
                }),
            ));
        }
        let counts = score(spec, &labels);
        results.push(json!({"threshold":value,"unit":spec.unit,"tp":counts.tp,"fp":counts.fp,"fn":counts.fn_,"tn":counts.tn,
            "precision":counts.precision(),"recall":counts.recall(),"false_high_observation_confidence":counts.false_high_observation,
            "false_high_causal_confidence":counts.false_high_causal,
            "primary_cause_agreement_with_current":stable as f64 / labeled.len() as f64,
            "observation_confidence_distribution":observation,
            "causal_confidence_distribution":causal}));
    }
    // Regression-only counts from the legacy name/source inference; never used for sufficiency.
    let legacy = |positive: bool| {
        labeled
            .iter()
            .filter(|f| f.legacy_real && f.label(spec) == Some(positive))
            .count()
    };
    let floors = family_floors(spec, &fixtures);
    let decision = if floors.sufficient() {
        "evidence_sufficient_for_review"
    } else {
        "retain_current_threshold_evidence_insufficient"
    };
    let report = json!({"schema_version":2,"family":spec.name,"corpus_id":frozen_id,"current_threshold":spec.current,"unit":spec.unit,
        "corpus":{"labeled":labeled.len(),"unlabeled":fixtures.len() - labeled.len(),
            "legacy_inferred_real_positive":legacy(true),"legacy_inferred_real_negative":legacy(false)},
        "sufficiency":{"positives":floors.positives,"negative_controls":floors.negative_controls,
            "mixed_confounded":floors.mixed_confounded,"ineligible_real_captures":floors.ineligible_real,
            "duplicate_capture_ids":floors.duplicate_capture_ids,"sufficient":floors.sufficient()},
        "evidence_floors":{"positive":MIN_REAL_POSITIVES,"negative_control":MIN_REAL_NEGATIVES,"mixed_confounded":MIN_MIXED},
        "sweep":results,"decision":decision,
        "limitations":["family-level labels; specific-cause agreement is not scored",
            "sufficiency counts only captures with an explicit non-synthetic evidence_kind, complete provenance, an explicit control_type and [coverage].diagnoses naming the family; legacy inferred real fixtures run in the sweep but cannot satisfy a floor",
            "other families' positives are not negative controls; a mixed capture counts only for the families it names",
            "observation and causal confidence are reported separately; causal confidence is capped by the explicit-attribution-chain policy and visible-stutter linkage, and no causal threshold is tuned here",
            "overlap is not proven causality: IRQ/I-O overlap fixtures do not calibrate causal claims",
            "numerical optimum is never chosen mechanically; false high-confidence claims weigh more than inconclusive ones"]});
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(out) = out {
        fs::write(root.join(out), serde_json::to_vec_pretty(&report)?)?;
    }
    if !FAMILIES.iter().any(|f| f.name == family_name) {
        bail!("unreachable family");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_the_family_and_skip_wildcards() {
        let text = "name = \"real_x\"\nsource = \"sanitized-real-recording\"\n[expected]\nprimary_cause = \"GameThreadSchedulerDelay\"\n";
        let fixture = parse_fixture(Path::new("d"), text);
        assert!(fixture.legacy_real);
        let (scheduler, io) = (
            family_spec("scheduler").unwrap(),
            family_spec("io").unwrap(),
        );
        assert_eq!(fixture.label(scheduler), Some(true));
        assert_eq!(fixture.label(io), Some(false));
        let control = parse_fixture(
            Path::new("d"),
            "name = \"c\"\n[expected]\nprimary_cause = \"IrqDelayCandidate\"\n[platform]\nscenario = \"mixed-io\"\n",
        );
        assert_eq!(
            (control.label(scheduler), control.legacy_real),
            (Some(false), false)
        );
        assert_eq!(control.label(family_spec("irq").unwrap()), Some(true));
        assert_eq!(
            parse_fixture(
                Path::new("d"),
                "name = \"a\"\n[expected]\nprimary_cause = \"Any\"\n"
            )
            .label(scheduler),
            None
        );
        assert!(family_spec("thermal").is_err());
    }

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
        assert_eq!(
            fixtures.iter().filter(|f| f.meta.is_some()).count(),
            0,
            "no committed capture is calibration-eligible yet"
        );
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

    /// A complete explicit capture. `diagnoses` uses the multi-line array form on purpose.
    fn capture(kind: &str, control: &str, diagnoses: &[&str], id: &str) -> Fixture {
        let provenance = crate::fixture_coverage::REQUIRED_PROVENANCE
            .iter()
            .filter(|key| !matches!(**key, "control_type" | "capture_id"))
            .map(|key| format!("{key} = \"x\"\n"))
            .collect::<String>();
        let list = diagnoses
            .iter()
            .map(|d| format!("    \"{d}\",\n"))
            .collect::<String>();
        let text = format!(
            "name = \"neutral_{id}\"\nevidence_kind = \"{kind}\"\n[expected]\nprimary_cause = \"{}\"\n\
             [provenance]\n{provenance}control_type = \"{control}\"\ncapture_id = \"{id}\"\n\
             [coverage]\ndiagnoses = [\n{list}]\n",
            diagnoses.first().copied().unwrap_or("Unknown")
        );
        parse_fixture(Path::new("d"), &text)
    }
    fn many(count: usize, control: &str, diagnoses: &[&str], prefix: &str) -> Vec<Fixture> {
        (0..count)
            .map(|i| capture("fixture_real", control, diagnoses, &format!("{prefix}-{i}")))
            .collect()
    }
    const SCHED: &str = "GameThreadSchedulerDelay";

    #[test]
    fn a_real_filename_with_an_explicit_synthetic_kind_is_not_real_for_sufficiency() {
        let text = "name = \"real_looks_real\"\nsource = \"sanitized-real-recording\"\nevidence_kind = \"synthetic\"\n";
        let fixture = parse_fixture(Path::new("d"), text);
        assert!(fixture.legacy_real && fixture.meta.is_none());
        assert!(fixture.ineligible_reason.contains("synthetic"));
        let mut explicit = capture("synthetic", "positive", &[SCHED], "s-0");
        assert!(explicit.meta.is_none());
        explicit.legacy_real = true;
        assert_eq!(
            family_floors(family_spec("scheduler").unwrap(), &[explicit]).positives,
            0
        );
    }

    #[test]
    fn a_neutral_filename_with_an_explicit_real_kind_is_eligible() {
        let fixture = capture("fixture_real", "positive", &[SCHED], "n-0");
        assert!(!fixture.legacy_real);
        let meta = fixture.meta.clone().expect("eligible");
        assert_eq!(meta.control_type, "positive");
        assert_eq!(meta.diagnoses, vec![SCHED.to_owned()]);
        let floors = family_floors(family_spec("scheduler").unwrap(), &[fixture]);
        assert_eq!(floors.positives, 1);
    }

    #[test]
    fn legacy_inferred_real_fixtures_cannot_satisfy_the_floor() {
        let legacy = (0..20)
            .map(|i| {
                parse_fixture(
                    Path::new("d"),
                    &format!(
                        "name = \"real_{i}\"\nsource = \"sanitized-real-recording\"\n[expected]\nprimary_cause = \"{SCHED}\"\n"
                    ),
                )
            })
            .collect::<Vec<_>>();
        let floors = family_floors(family_spec("scheduler").unwrap(), &legacy);
        assert_eq!((floors.positives, floors.ineligible_real), (0, 20));
        assert!(!floors.sufficient());
    }

    #[test]
    fn incomplete_provenance_or_control_type_is_ineligible() {
        let missing = parse_fixture(
            Path::new("d"),
            "name = \"x\"\nevidence_kind = \"fixture_real\"\n[provenance]\ncontrol_type = \"positive\"\n[coverage]\ndiagnoses = [\"GameThreadSchedulerDelay\"]\n",
        );
        assert!(missing.ineligible_reason.contains("[provenance]."));
        let mut bad = capture("fixture_real", "positive", &[SCHED], "b-0");
        assert!(bad.meta.is_some());
        let text = "name = \"y\"\nevidence_kind = \"fixture_real\"\n";
        bad = parse_fixture(Path::new("d"), text);
        assert!(bad.meta.is_none());
        let wrong = capture("fixture_real", "maybe", &[SCHED], "w-0");
        assert!(wrong.ineligible_reason.contains("control_type"));
        let no_coverage = parse_fixture(
            Path::new("d"),
            &format!(
                "name = \"z\"\nevidence_kind = \"fixture_real\"\n[provenance]\n{}control_type = \"positive\"\n",
                crate::fixture_coverage::REQUIRED_PROVENANCE
                    .iter()
                    .filter(|k| **k != "control_type")
                    .map(|k| format!("{k} = \"x\"\n"))
                    .collect::<String>()
            ),
        );
        assert!(no_coverage.ineligible_reason.contains("diagnoses"));
    }

    #[test]
    fn another_familys_positive_is_not_a_negative_control() {
        let gpu = capture("fixture_real", "positive", &["GpuBoundCandidate"], "g-0");
        let scheduler = family_floors(family_spec("scheduler").unwrap(), &[gpu.clone()]);
        assert_eq!((scheduler.positives, scheduler.negative_controls), (0, 0));
        let gpu_floors = family_floors(family_spec("gpu").unwrap(), &[gpu]);
        assert_eq!(gpu_floors.positives, 1);
    }

    #[test]
    fn an_explicit_negative_control_counts_only_for_the_family_it_names() {
        let negative = capture("fixture_real", "negative_control", &[SCHED], "neg-0");
        let floors = |name| family_floors(family_spec(name).unwrap(), &[negative.clone()]);
        assert_eq!(floors("scheduler").negative_controls, 1);
        assert_eq!(floors("io").negative_controls, 0);
    }

    #[test]
    fn a_mixed_capture_counts_only_for_the_families_it_names() {
        let mixed = capture(
            "fixture_real",
            "mixed_confounded",
            &[SCHED, "BlockIoCandidate"],
            "m-0",
        );
        let floors = |name| family_floors(family_spec(name).unwrap(), &[mixed.clone()]);
        assert_eq!(floors("scheduler").mixed_confounded, 1);
        assert_eq!(floors("io").mixed_confounded, 1);
        assert_eq!(floors("irq").mixed_confounded, 0);
        assert_eq!(floors("gpu").mixed_confounded, 0);
    }

    #[test]
    fn three_scheduler_mixed_captures_cannot_satisfy_other_families_floors() {
        let mut fixtures = many(3, "mixed_confounded", &[SCHED], "sm");
        for family in ["irq", "gpu", "io"] {
            let floors = family_floors(family_spec(family).unwrap(), &fixtures);
            assert_eq!(floors.mixed_confounded, 0, "{family}");
        }
        fixtures.extend(many(10, "positive", &[SCHED], "sp"));
        fixtures.extend(many(10, "negative_control", &[SCHED], "sn"));
        let scheduler = family_floors(family_spec("scheduler").unwrap(), &fixtures);
        assert_eq!(
            (
                scheduler.positives,
                scheduler.negative_controls,
                scheduler.mixed_confounded
            ),
            (10, 10, 3)
        );
        assert!(scheduler.sufficient());
        assert!(!family_floors(family_spec("irq").unwrap(), &fixtures).sufficient());
    }

    #[test]
    fn duplicate_capture_ids_are_counted_once() {
        let mut fixtures = many(10, "positive", &[SCHED], "dup");
        fixtures.push(capture("fixture_real", "positive", &[SCHED], "dup-3"));
        let floors = family_floors(family_spec("scheduler").unwrap(), &fixtures);
        assert_eq!((floors.positives, floors.duplicate_capture_ids), (10, 1));
    }
}
