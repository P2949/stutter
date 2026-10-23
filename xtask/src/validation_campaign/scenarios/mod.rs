//! Deterministic scenarios: model, corpus accounting, capture acceptance and fixture
//! generation. The campaign computes corpus deficits itself; the agent never does.
pub mod capture;
pub mod exec;
pub mod fixture;
pub mod model;
pub mod probe;

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Value, json};

use self::{
    capture::{NextAction, next_action},
    model::{Family, ScenarioSet},
};
use crate::diagnosis_calibration::{FAMILIES, Fixture, family_floors};

/// Progress of one scenario toward its requested captures.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScenarioProgress {
    pub id: String,
    pub have: usize,
    pub need: usize,
    pub failed: usize,
}

/// Where the corpus stands, derived from the fixtures that are actually eligible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CorpusPlan {
    pub scenarios: Vec<ScenarioProgress>,
    failed_per_capture: usize,
}

impl CorpusPlan {
    /// `failed` is the failed-attempt count per scenario id recorded by the campaign.
    pub fn from_fixtures(
        set: &ScenarioSet,
        fixtures: &[Fixture],
        failed: &BTreeMap<String, usize>,
        failed_per_capture: usize,
    ) -> Self {
        let mut seen = BTreeSet::new();
        let mut have: BTreeMap<&str, usize> = BTreeMap::new();
        for fixture in fixtures {
            let Some(meta) = &fixture.meta else { continue };
            // A repeated capture id counts once, and only known scenarios count.
            if set.get(&fixture.scenario).is_some() && seen.insert(meta.capture_id.clone()) {
                *have.entry(fixture.scenario.as_str()).or_default() += 1;
            }
        }
        let scenarios = set
            .scenarios
            .iter()
            .map(|spec| ScenarioProgress {
                id: spec.id.clone(),
                have: have.get(spec.id.as_str()).copied().unwrap_or(0),
                need: spec.count,
                failed: failed.get(&spec.id).copied().unwrap_or(0),
            })
            .collect();
        Self {
            scenarios,
            failed_per_capture,
        }
    }

    pub fn complete(&self) -> bool {
        self.scenarios.iter().all(|s| s.have >= s.need)
    }

    /// Scenarios (repeated once per missing capture) the campaign should run next, skipping
    /// any whose retry budget is spent.
    pub fn next_scenarios(&self, limit: usize) -> Vec<String> {
        self.scenarios
            .iter()
            .filter(|s| {
                matches!(
                    next_action(s.have, s.need, s.failed, self.failed_per_capture),
                    NextAction::Capture
                )
            })
            .flat_map(|s| std::iter::repeat_n(s.id.clone(), s.need - s.have))
            .take(limit)
            .collect()
    }

    /// The first scenario whose retry budget is exhausted while captures are still missing.
    pub fn exhausted(&self) -> Option<String> {
        self.scenarios.iter().find_map(|s| match next_action(s.have, s.need, s.failed, self.failed_per_capture) {
            NextAction::Exhausted { failed, limit } => Some(format!(
                "scenario {} has {} of {} captures after {failed} failed attempts (limit {limit})",
                s.id, s.have, s.need
            )),
            _ => None,
        })
    }
}

/// Per-family have/need for `status --json`, from the same floors the calibrator enforces.
pub fn family_summary(
    fixtures: &[Fixture],
    min: (usize, usize, usize),
    plan: &CorpusPlan,
) -> Value {
    let families = FAMILIES
        .iter()
        .map(|spec| {
            let floors = family_floors(spec, fixtures);
            (
                spec.name.to_owned(),
                json!({
                    "positive": {"have": floors.positives, "need": min.0},
                    "negative_control": {"have": floors.negative_controls, "need": min.1},
                    "mixed_confounded": {"have": floors.mixed_confounded, "need": min.2},
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let mut summary = families;
    summary.insert("next_scenarios".to_owned(), json!(plan.next_scenarios(6)));
    Value::Object(summary)
}

/// Families a fixture set can still be short in; used by the corpus runner for reporting.
pub fn short_families(fixtures: &[Fixture], min: (usize, usize, usize)) -> Vec<Family> {
    Family::ALL
        .into_iter()
        .filter(|family| {
            FAMILIES
                .iter()
                .find(|s| s.name == family.as_str())
                .is_some_and(|spec| {
                    let floors = family_floors(spec, fixtures);
                    floors.positives < min.0
                        || floors.negative_controls < min.1
                        || floors.mixed_confounded < min.2
                })
        })
        .collect()
}
