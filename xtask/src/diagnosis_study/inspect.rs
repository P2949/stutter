//! Read-only views of a study: machine-readable status, strict verification and the report.
//! Everything is recomputed from the retained logs and files.
use std::{collections::BTreeSet, fs};

use serde_json::{Value, json};

use super::{
    model::StudyConfig,
    plan::{Action, family_passed, next_action, target_pairs},
    store::{Decision, Outcome, Role, RunRecord, StudyDir, StudyHeader},
};
use crate::{evidence_bundle::sha256::sha256_hex, validation_campaign::scenarios::model::Family};

pub const COMPLETE_MARKER: &str = "complete.json";

fn outcome_name(decisions: &[Decision], family: Family) -> &'static str {
    match decisions.iter().rev().find(|d| d.family == family) {
        None => "pending",
        Some(d) => match d.outcome {
            Outcome::Pass => "pass",
            Outcome::Extend => "extend",
            Outcome::Invalid => "invalid",
        },
    }
}

fn accepted_of(runs: &[RunRecord], scenario: &str) -> usize {
    runs.iter()
        .filter(|r| r.accepted && r.scenario == scenario)
        .count()
}

fn rejected_of(runs: &[RunRecord], scenario: &str) -> usize {
    runs.iter()
        .filter(|r| !r.accepted && r.scenario == scenario)
        .count()
}

pub fn status(
    study: &StudyDir,
    header: &StudyHeader,
    config: &StudyConfig,
) -> anyhow::Result<Value> {
    let (runs, decisions) = (study.runs()?, study.decisions()?);
    let next = next_action(config, &runs, &decisions);
    let complete = study.0.join(COMPLETE_MARKER).is_file();
    let mut families = serde_json::Map::new();
    for family in Family::ALL {
        let control = super::plan::control_id(family);
        let positive = super::plan::positive_id(family);
        families.insert(
            family.as_str().to_owned(),
            json!({
                "decision": outcome_name(&decisions, family),
                "target_pairs": target_pairs(config, &decisions, family),
                "accepted_controls": accepted_of(&runs, &control),
                "accepted_positives": accepted_of(&runs, &positive),
                "rejected": rejected_of(&runs, &control) + rejected_of(&runs, &positive),
            }),
        );
    }
    let mixed: Vec<Value> = config
        .mixed
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "eligible": m.covers.iter().all(|f| family_passed(&decisions, *f)),
                "accepted": accepted_of(&runs, &m.id),
                "needed": m.count,
                "rejected": rejected_of(&runs, &m.id),
            })
        })
        .collect();
    let (next_json, blocked) = match &next {
        Action::Capture(slot) => (
            json!({"action": "capture", "scenario": slot.scenario, "pair": slot.pair}),
            None,
        ),
        Action::Evaluate { family, pairs } => (
            json!({"action": "evaluate", "family": family.as_str(), "pairs": pairs}),
            None,
        ),
        Action::Blocked(reason) => (json!({"action": "blocked"}), Some(reason.clone())),
        Action::Done => (json!({"action": "done"}), None),
    };
    Ok(json!({
        "study_id": header.study_id,
        "base_commit": header.base_commit,
        "companion_campaign": header.companion_campaign,
        "complete": complete,
        "blocked": blocked,
        "next": next_json,
        "frozen": header.frozen,
        "families": families,
        "mixed": mixed,
        "runs": runs.len(),
    }))
}

/// Every inconsistency found; empty means the study verifies.
pub fn verify(study: &StudyDir, header: &StudyHeader, config: &StudyConfig) -> Vec<String> {
    let mut problems = Vec::new();
    let runs = study.runs().unwrap_or_default();
    let decisions = study.decisions().unwrap_or_default();
    if sha256_hex(&fs::read(study.0.join("config.toml")).unwrap_or_default())
        != header.config_sha256
    {
        problems.push("the retained study definition does not match the header digest".to_owned());
    }
    let mut capture_ids = BTreeSet::new();
    for run in runs.iter().filter(|r| r.accepted) {
        let Some(name) = &run.fixture else {
            problems.push(format!("run {} is accepted without a fixture", run.seq));
            continue;
        };
        let behavior = fs::read_to_string(study.fixtures().join(name).join("fixture.toml"))
            .ok()
            .and_then(|t| t.parse::<toml::Table>().ok())
            .and_then(|t| {
                t.get("expected")?
                    .get("expected_behavior")?
                    .as_str()
                    .map(str::to_owned)
            });
        match behavior {
            Some(b) if b == "informational" => {}
            Some(_) => problems.push(format!(
                "{name}: a v2 calibration label must be informational"
            )),
            None => problems.push(format!("{name}: fixture.toml is missing or unreadable")),
        }
        if !capture_ids.insert(run.capture_id.clone()) {
            problems.push(format!("{name}: duplicate capture id"));
        }
        if run.family.is_some()
            && run.role != Role::Mixed
            && !run.metrics["primary"].as_f64().is_some_and(f64::is_finite)
        {
            problems.push(format!(
                "{name}: an accepted family run has no primary metric"
            ));
        }
    }
    for decision in &decisions {
        let path = study.impact().join(format!(
            "{}-{}.json",
            decision.family.as_str(),
            decision.pairs
        ));
        match fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            Some(mut report) => {
                report.as_object_mut().map(|o| o.remove("report_sha256"));
                if sha256_hex(report.to_string().as_bytes()) != decision.report_sha256 {
                    problems.push(format!("{}: impact report digest differs", path.display()));
                }
            }
            None => problems.push(format!("{}: impact report missing", path.display())),
        }
    }
    // Promoted fixtures belong only to families whose impact study passed.
    let promoted = study.calibration().join("fixtures");
    for entry in fs::read_dir(&promoted).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let family = runs
            .iter()
            .find(|r| r.fixture.as_deref() == Some(&name))
            .map(|r| (r.family, r.role));
        let allowed = match family {
            Some((Some(family), role)) if role != Role::Mixed => family_passed(&decisions, family),
            Some((_, Role::Mixed)) => true,
            _ => false,
        };
        if !allowed {
            problems.push(format!("{name}: promoted without a passed impact study"));
        }
    }
    let _ = config;
    problems
}

pub fn report(
    study: &StudyDir,
    header: &StudyHeader,
    config: &StudyConfig,
) -> anyhow::Result<Value> {
    let status = status(study, header, config)?;
    let decisions = study.decisions()?;
    let runs = study.runs()?;
    let mut rejections: std::collections::BTreeMap<String, usize> = Default::default();
    for run in runs.iter().filter(|r| !r.accepted) {
        *rejections
            .entry(format!(
                "{}:{}",
                run.scenario,
                run.stage.as_deref().unwrap_or("unknown")
            ))
            .or_default() += 1;
    }
    let mut impacts = serde_json::Map::new();
    for family in Family::ALL {
        if let Some(decision) = decisions.iter().rev().find(|d| d.family == family) {
            let path = study
                .impact()
                .join(format!("{}-{}.json", family.as_str(), decision.pairs));
            let report: Value = fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(Value::Null);
            impacts.insert(
                family.as_str().to_owned(),
                json!({
                    "outcome": decision.outcome,
                    "pairs": decision.pairs,
                    "report_sha256": decision.report_sha256,
                    "effect_size": report["comparison"]["effect_size"],
                    "improvement_delta": report["comparison"]["improvement_delta"],
                    "bootstrap_ci95": report["comparison"]["bootstrap_ci95"],
                    "control_median": report["comparison"]["baseline_median"],
                    "positive_median": report["comparison"]["tuned_median"],
                }),
            );
        }
    }
    let report = json!({
        "schema_version": 1,
        "study": status,
        "target_impact": impacts,
        "rejections": rejections,
        "limitations": [
            "single host; ground truth is the predeclared intervention and its independent verification",
            "the direct effect metric is the target's own runnable p99, frame p99 or fixed-work elapsed time, never a diagnosis output",
            "captures of a family enter calibration only if its target-impact study passed",
            "the privileged runner is authorized through a companion core-validation campaign; no release task of it is run",
        ],
    });
    fs::write(
        study.0.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
