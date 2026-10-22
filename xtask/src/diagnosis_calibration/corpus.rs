//! Calibration corpus model: fixtures, their explicit eligibility, per-family roles and floors.
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{FamilySpec, MIN_MIXED, MIN_REAL_NEGATIVES, MIN_REAL_POSITIVES};
use crate::{
    evidence_bundle::sha256::sha256_hex,
    fixture_coverage::{CalibrationMeta, parse_fixture_toml_text},
};

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

/// A capture's role in one family's calibration decision data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalibrationRole {
    Positive,
    NegativeControl,
    /// Evaluated in its own section, never as an ordinary negative or positive.
    MixedConfounded,
    Ineligible,
}

impl Fixture {
    /// Derived only from explicit metadata: `control_type`, `[coverage].diagnoses` and the
    /// family. A positive must also expect a cause that belongs to the family.
    pub fn role(&self, spec: &FamilySpec) -> CalibrationRole {
        let Some(meta) = &self.meta else {
            return CalibrationRole::Ineligible;
        };
        if !Self::names_family(meta, spec) {
            return CalibrationRole::Ineligible;
        }
        match meta.control_type.as_str() {
            "positive" if spec.positives.contains(&self.expected_cause.as_str()) => {
                CalibrationRole::Positive
            }
            "negative_control" => CalibrationRole::NegativeControl,
            "mixed_confounded" => CalibrationRole::MixedConfounded,
            _ => CalibrationRole::Ineligible,
        }
    }
}

/// The decision dataset for one family: captures with a non-ineligible role, a repeated
/// capture id counted once (first wins). Returns the duplicates skipped.
pub fn decision_set<'a>(
    spec: &FamilySpec,
    fixtures: &'a [Fixture],
) -> (Vec<(&'a Fixture, CalibrationRole)>, usize) {
    let mut seen = std::collections::BTreeSet::new();
    let (mut set, mut duplicates) = (Vec::new(), 0);
    for fixture in fixtures {
        let role = fixture.role(spec);
        let Some(meta) = fixture
            .meta
            .as_ref()
            .filter(|_| role != CalibrationRole::Ineligible)
        else {
            continue;
        };
        if seen.insert(meta.capture_id.clone()) {
            set.push((fixture, role));
        } else {
            duplicates += 1;
        }
    }
    (set, duplicates)
}

/// Counts evidence floors for one family from its decision dataset; real-looking captures
/// that cannot count (legacy inference, synthetic, incomplete, another family's) are reported
/// as `ineligible_real`.
pub fn family_floors(spec: &FamilySpec, fixtures: &[Fixture]) -> Floors {
    let (set, duplicates) = decision_set(spec, fixtures);
    let mut floors = Floors {
        duplicate_capture_ids: duplicates,
        ..Floors::default()
    };
    for (_, role) in &set {
        match role {
            CalibrationRole::Positive => floors.positives += 1,
            CalibrationRole::NegativeControl => floors.negative_controls += 1,
            CalibrationRole::MixedConfounded => floors.mixed_confounded += 1,
            CalibrationRole::Ineligible => {}
        }
    }
    floors.ineligible_real = fixtures
        .iter()
        .filter(|f| f.meta.is_none() && f.legacy_real)
        .count();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnosis_calibration::family_spec;

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
        let scheduler = family_floors(
            family_spec("scheduler").unwrap(),
            std::slice::from_ref(&gpu),
        );
        assert_eq!((scheduler.positives, scheduler.negative_controls), (0, 0));
        let gpu_floors = family_floors(family_spec("gpu").unwrap(), &[gpu]);
        assert_eq!(gpu_floors.positives, 1);
    }

    #[test]
    fn an_explicit_negative_control_counts_only_for_the_family_it_names() {
        let negative = capture("fixture_real", "negative_control", &[SCHED], "neg-0");
        let floors =
            |name| family_floors(family_spec(name).unwrap(), std::slice::from_ref(&negative));
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
        let floors = |name| family_floors(family_spec(name).unwrap(), std::slice::from_ref(&mixed));
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

    fn decision<'a>(
        family: &str,
        fixtures: &'a [Fixture],
    ) -> (Vec<(&'a Fixture, bool)>, Vec<&'a Fixture>) {
        let (set, _) = decision_set(family_spec(family).unwrap(), fixtures);
        crate::diagnosis_calibration::sweep::partition(&set)
    }

    #[test]
    fn a_gpu_positive_is_absent_from_the_scheduler_decision_labels() {
        let gpu = [capture(
            "fixture_real",
            "positive",
            &["GpuBoundCandidate"],
            "g-0",
        )];
        let (binary, mixed) = decision("scheduler", &gpu);
        assert!(binary.is_empty() && mixed.is_empty());
        assert_eq!(decision("gpu", &gpu).0.len(), 1);
    }

    #[test]
    fn a_scheduler_negative_control_is_a_scheduler_negative_only() {
        let negative = [capture("fixture_real", "negative_control", &[SCHED], "n-0")];
        let (binary, _) = decision("scheduler", &negative);
        assert_eq!(binary.len(), 1);
        assert!(!binary[0].1, "labelled negative");
        assert!(decision("io", &negative).0.is_empty());
    }

    #[test]
    fn a_mixed_capture_is_never_an_ordinary_negative_and_appears_for_both_named_families() {
        let mixed = [capture(
            "fixture_real",
            "mixed_confounded",
            &[SCHED, "BlockIoCandidate"],
            "m-0",
        )];
        for family in ["scheduler", "io"] {
            let (binary, mixed_list) = decision(family, &mixed);
            assert!(binary.is_empty(), "{family}: mixed must not be binary data");
            assert_eq!(mixed_list.len(), 1, "{family}");
        }
        for family in ["irq", "gpu"] {
            let (binary, mixed_list) = decision(family, &mixed);
            assert!(binary.is_empty() && mixed_list.is_empty(), "{family}");
        }
    }

    #[test]
    fn a_positive_must_expect_a_cause_of_its_own_family() {
        // Declared control_type positive and covering scheduler, but expecting an I/O cause.
        let mut odd = capture("fixture_real", "positive", &[SCHED], "o-0");
        odd.expected_cause = "BlockIoCandidate".to_owned();
        assert_eq!(
            odd.role(family_spec("scheduler").unwrap()),
            CalibrationRole::Ineligible
        );
    }

    #[test]
    fn legacy_real_fixtures_never_enter_the_decision_dataset() {
        let legacy = [parse_fixture(
            Path::new("d"),
            &format!(
                "name = \"real_x\"\nsource = \"sanitized-real-recording\"\n[expected]\nprimary_cause = \"{SCHED}\"\n"
            ),
        )];
        let (binary, mixed) = decision("scheduler", &legacy);
        assert!(binary.is_empty() && mixed.is_empty());
        assert_eq!(
            legacy[0].role(family_spec("scheduler").unwrap()),
            CalibrationRole::Ineligible
        );
    }

    #[test]
    fn a_repeated_capture_id_enters_the_decision_set_once() {
        let twice = [
            capture("fixture_real", "positive", &[SCHED], "dup-0"),
            capture("fixture_real", "positive", &[SCHED], "dup-0"),
        ];
        let (set, duplicates) = decision_set(family_spec("scheduler").unwrap(), &twice);
        assert_eq!((set.len(), duplicates), (1, 1));
    }
}
