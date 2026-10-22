//! Evidence-maturity coverage by diagnosis, objective, mutation provider and kernel
//! capability. Denominators come from the code registries, never from this file, and
//! maturity is reported per category instead of as a single percentage.
use std::{fs, path::Path};

use anyhow::{Context, ensure};
use serde_json::Value;

use crate::evidence_bundle::EvidenceKind;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Tags {
    pub diagnoses: Vec<String>,
    pub objectives: Vec<String>,
    pub providers: Vec<String>,
    pub kernel_capabilities: Vec<String>,
}

/// One piece of evidence (a fixture or a verified bundle) and what it covers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation {
    pub kind: EvidenceKind,
    pub tags: Tags,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registries {
    pub diagnoses: Vec<String>,
    pub objectives: Vec<String>,
    pub providers: Vec<String>,
    pub kernel_capabilities: Vec<String>,
}

impl Registries {
    pub fn from_code() -> Self {
        Self {
            diagnoses: stutter::api::diagnosis::StutterCause::ALL
                .iter()
                .map(|cause| format!("{cause:?}"))
                .collect(),
            objectives: stutter::api::autotune::ObjectiveKind::ALL
                .iter()
                .map(|kind| format!("{kind:?}"))
                .collect(),
            providers: stutter::api::autotune::candidate::MUTATION_PROVIDER_KINDS
                .iter()
                .map(|kind| (*kind).to_owned())
                .collect(),
            kernel_capabilities: stutter::api::probe_registry::implemented_probe_specs()
                .map(|spec| spec.catalog_key.to_owned())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub name: String,
    /// Indexed like [`EvidenceKind::ALL`].
    pub counts: [usize; 4],
}
impl Row {
    pub fn count(&self, kind: EvidenceKind) -> usize {
        self.counts[EvidenceKind::ALL
            .iter()
            .position(|k| *k == kind)
            .unwrap_or(0)]
    }
    /// Real-evidence debt: only synthetic evidence (or none) covers this surface.
    pub fn has_real_evidence_debt(&self) -> bool {
        EvidenceKind::ALL
            .iter()
            .all(|kind| *kind == EvidenceKind::Synthetic || self.count(*kind) == 0)
    }
    /// No evidence from a real workload (as opposed to fixtures or controlled runs).
    pub fn has_real_workload_debt(&self) -> bool {
        self.count(EvidenceKind::RealWorkload) == 0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MaturityReport {
    pub diagnoses: Vec<Row>,
    pub objectives: Vec<Row>,
    pub providers: Vec<Row>,
    pub kernel_capabilities: Vec<Row>,
    pub legacy_inferred_fixtures: usize,
    pub tag_errors: Vec<String>,
}

fn rows(
    dimension: &str,
    registry: &[String],
    observations: &[Observation],
    select: fn(&Tags) -> &Vec<String>,
    errors: &mut Vec<String>,
) -> Vec<Row> {
    let mut rows = registry
        .iter()
        .map(|name| Row {
            name: name.clone(),
            counts: [0; 4],
        })
        .collect::<Vec<_>>();
    for observation in observations {
        let slot = EvidenceKind::ALL
            .iter()
            .position(|kind| *kind == observation.kind)
            .unwrap_or(0);
        for tag in select(&observation.tags) {
            match rows.iter_mut().find(|row| &row.name == tag) {
                Some(row) => row.counts[slot] += 1,
                None => errors.push(format!("{dimension}: unknown tag {tag:?}")),
            }
        }
    }
    rows
}

pub fn build_report(
    registries: &Registries,
    observations: &[Observation],
    legacy_inferred_fixtures: usize,
) -> MaturityReport {
    let mut errors = Vec::new();
    MaturityReport {
        diagnoses: rows(
            "diagnosis",
            &registries.diagnoses,
            observations,
            |t| &t.diagnoses,
            &mut errors,
        ),
        objectives: rows(
            "objective",
            &registries.objectives,
            observations,
            |t| &t.objectives,
            &mut errors,
        ),
        providers: rows(
            "provider",
            &registries.providers,
            observations,
            |t| &t.providers,
            &mut errors,
        ),
        kernel_capabilities: rows(
            "kernel_capability",
            &registries.kernel_capabilities,
            observations,
            |t| &t.kernel_capabilities,
            &mut errors,
        ),
        legacy_inferred_fixtures,
        tag_errors: errors,
    }
}

impl MaturityReport {
    pub fn dimensions(&self) -> [(&'static str, &[Row]); 4] {
        [
            ("diagnosis", &self.diagnoses),
            ("objective", &self.objectives),
            ("mutation provider", &self.providers),
            ("kernel capability", &self.kernel_capabilities),
        ]
    }

    pub fn debt(&self) -> Vec<String> {
        self.dimensions()
            .iter()
            .flat_map(|(dimension, rows)| {
                rows.iter()
                    .filter(|row| row.has_real_evidence_debt())
                    .map(move |row| format!("{dimension}:{}", row.name))
            })
            .collect()
    }

    pub fn workload_debt(&self) -> Vec<String> {
        self.dimensions()
            .iter()
            .flat_map(|(dimension, rows)| {
                rows.iter()
                    .filter(|row| row.has_real_workload_debt())
                    .map(move |row| format!("{dimension}:{}", row.name))
            })
            .collect()
    }

    pub fn render_text(&self) -> String {
        let mut text = String::from("evidence maturity (categories, not a percentage)\n");
        for (dimension, rows) in self.dimensions() {
            text.push_str(&format!("  {dimension} ({} in registry)\n", rows.len()));
            for row in rows {
                text.push_str(&format!("    {}:", row.name));
                for kind in EvidenceKind::ALL {
                    text.push_str(&format!(" {}={}", kind.as_str(), row.count(kind)));
                }
                text.push('\n');
            }
        }
        text.push_str(&format!(
            "  fixtures with legacy (inferred) evidence kind: {}\n",
            self.legacy_inferred_fixtures
        ));
        let list = |items: Vec<String>| {
            if items.is_empty() {
                "none".to_owned()
            } else {
                items.join(", ")
            }
        };
        let debt = self.debt();
        text.push_str(&format!(
            "synthetic-only or uncovered ({}): {}\n",
            debt.len(),
            list(debt)
        ));
        let workload = self.workload_debt();
        text.push_str(&format!(
            "no real_workload evidence ({}): {}\n",
            workload.len(),
            list(workload)
        ));
        text
    }
}

fn string_list(value: &Value, name: &str) -> Vec<String> {
    value["covers"][name]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Verified-bundle index (`docs/evidence/index.json`). Only PASS entries count.
pub fn read_evidence_index(path: &Path) -> anyhow::Result<Vec<Observation>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let index: Value = serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    ensure!(
        index["schema_version"] == 1,
        "unsupported evidence index schema"
    );
    let mut observations = Vec::new();
    for entry in index["entries"]
        .as_array()
        .context("entries must be an array")?
    {
        for required in ["suite", "capture_id", "source_commit", "manifest_sha256"] {
            ensure!(entry[required].is_string(), "index entry lacks {required}");
        }
        let kind = EvidenceKind::parse(entry["evidence_kind"].as_str().unwrap_or(""))?;
        if entry["status"] != "PASS" {
            continue;
        }
        observations.push(Observation {
            kind,
            tags: Tags {
                diagnoses: string_list(entry, "diagnoses"),
                objectives: string_list(entry, "objectives"),
                providers: string_list(entry, "providers"),
                kernel_capabilities: string_list(entry, "kernel_capabilities"),
            },
        });
    }
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registries() -> Registries {
        Registries {
            diagnoses: vec!["IrqDelayCandidate".into()],
            objectives: vec!["GameRunnableLatency".into(), "IoLatency".into()],
            providers: vec!["cpu_affinity_profile".into(), "nice".into()],
            kernel_capabilities: vec!["sched_core".into()],
        }
    }
    fn observation(kind: EvidenceKind, objective: &str, provider: &str) -> Observation {
        Observation {
            kind,
            tags: Tags {
                objectives: vec![objective.into()],
                providers: vec![provider.into()],
                ..Tags::default()
            },
        }
    }

    #[test]
    fn categories_are_reported_per_registry_entry_and_debt_is_explicit() {
        let report = build_report(
            &registries(),
            &[
                observation(EvidenceKind::Synthetic, "GameRunnableLatency", "nice"),
                observation(
                    EvidenceKind::ControlledReal,
                    "GameRunnableLatency",
                    "cpu_affinity_profile",
                ),
            ],
            3,
        );
        let runnable = &report.objectives[0];
        assert_eq!(runnable.count(EvidenceKind::Synthetic), 1);
        assert_eq!(runnable.count(EvidenceKind::ControlledReal), 1);
        assert_eq!(runnable.count(EvidenceKind::RealWorkload), 0);
        assert!(!runnable.has_real_evidence_debt());
        let debt = report.debt();
        assert!(debt.contains(&"objective:IoLatency".to_owned()));
        assert!(debt.contains(&"mutation provider:nice".to_owned()));
        assert!(debt.contains(&"kernel capability:sched_core".to_owned()));
        assert!(!debt.contains(&"mutation provider:cpu_affinity_profile".to_owned()));
        assert!(report.tag_errors.is_empty());
        assert!(
            report
                .render_text()
                .contains("GameRunnableLatency: synthetic=1 fixture_real=0 controlled_real=1")
        );
    }

    #[test]
    fn unknown_tags_are_errors_not_new_denominators() {
        let report = build_report(
            &registries(),
            &[observation(EvidenceKind::FixtureReal, "Typo", "nice")],
            0,
        );
        assert_eq!(
            report.tag_errors,
            vec!["objective: unknown tag \"Typo\"".to_owned()]
        );
        assert_eq!(report.objectives.len(), 2);
    }

    #[test]
    fn registries_come_from_code_and_are_complete() {
        let registries = Registries::from_code();
        assert_eq!(registries.objectives.len(), 9);
        assert_eq!(registries.diagnoses.len(), 9);
        assert_eq!(registries.providers.len(), 9);
        assert!(!registries.kernel_capabilities.is_empty());
        assert!(
            registries
                .providers
                .contains(&"cpu_affinity_profile".to_owned())
        );
    }

    #[test]
    fn index_counts_only_passing_entries_with_provenance() {
        let dir = std::env::temp_dir().join(format!("stutter-index-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("index.json");
        let entry = |status: &str| {
            serde_json::json!({"suite":"live_affinity","evidence_kind":"controlled_real","status":status,
                "capture_id":"c","source_commit":"0123456789abcdef0123456789abcdef01234567",
                "manifest_sha256":"0".repeat(64),"covers":{"providers":["cpu_affinity_profile"]}})
        };
        fs::write(
            &path,
            serde_json::json!({"schema_version":1,"entries":[entry("PASS"), entry("FAIL")]})
                .to_string(),
        )
        .unwrap();
        let observations = read_evidence_index(&path).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].kind, EvidenceKind::ControlledReal);
        fs::write(&path, r#"{"schema_version":1,"entries":[{"suite":"x"}]}"#).unwrap();
        assert!(read_evidence_index(&path).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
