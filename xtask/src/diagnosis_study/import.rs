//! Carrying valid evidence into a new study. A repair to one family's intervention (or to the
//! host) must not force recollecting families whose definition and evidence are unchanged: their
//! runs, decisions, impact reports and fixtures are imported, byte for byte, under a recorded
//! provenance. A family is importable only if its impact study passed and its definition is
//! identical in both studies.
use std::fs;

use anyhow::{Context, bail, ensure};

use super::{
    model::StudyConfig,
    plan::family_passed,
    promote::link_tree,
    store::{Import, Role, RunRecord, StudyDir, StudyHeader},
};
use crate::{evidence_bundle::sha256::sha256_hex, validation_campaign::scenarios::model::Family};

/// Does `family` mean exactly the same experiment in both configurations?
pub fn same_definition(old: &StudyConfig, new: &StudyConfig, family: Family) -> bool {
    let (Some(a), Some(b)) = (old.family(family), new.family(family)) else {
        return false;
    };
    a == b
        && old.vkmark == new.vkmark
        && old.sampling.min_effect_size == new.sampling.min_effect_size
        && old.sampling.seconds == new.sampling.seconds
        && old.control_spec(a) == new.control_spec(b)
        && old.positive_spec(a) == new.positive_spec(b)
        && (family != Family::Irq || old.irq_traffic == new.irq_traffic)
}

pub fn import_families(
    from: &StudyDir,
    from_header: &StudyHeader,
    from_config: &StudyConfig,
    into: &StudyDir,
    into_config: &StudyConfig,
    families: &[Family],
) -> anyhow::Result<Import> {
    let decisions = from.decisions()?;
    for family in families {
        ensure!(
            family_passed(&decisions, *family),
            "{} did not pass its impact study in {}; only passed families are imported",
            family.as_str(),
            from_header.study_id
        );
        ensure!(
            same_definition(from_config, into_config, *family),
            "{} is defined differently in the new study; it must be recollected",
            family.as_str()
        );
    }
    let wanted = |family: Option<Family>| family.is_some_and(|f| families.contains(&f));
    let runs: Vec<RunRecord> = from
        .runs()?
        .into_iter()
        .filter(|r| wanted(r.family) && r.role != Role::Mixed)
        .collect();
    let mut lines = String::new();
    for run in &runs {
        lines.push_str(&serde_json::to_string(run)?);
        lines.push('\n');
        into.append_run(run)?;
    }
    for decision in decisions.iter().filter(|d| families.contains(&d.family)) {
        into.append_decision(decision)?;
        let name = format!("{}-{}.json", decision.family.as_str(), decision.pairs);
        let source = from.impact().join(&name);
        fs::create_dir_all(into.impact())?;
        fs::copy(&source, into.impact().join(&name))
            .with_context(|| format!("copy {}", source.display()))?;
    }
    for run in runs.iter().filter(|r| r.accepted) {
        let Some(name) = &run.fixture else {
            bail!("run {} is accepted without a fixture", run.seq);
        };
        link_tree(&from.fixtures().join(name), &into.fixtures().join(name))?;
        let promoted = from.calibration().join("fixtures").join(name);
        if promoted.is_dir() {
            link_tree(&promoted, &into.calibration().join("fixtures").join(name))?;
        }
    }
    Ok(Import {
        from_study: from_header.study_id.clone(),
        from_base_commit: from_header.base_commit.clone(),
        families: families.to_vec(),
        runs: runs.len(),
        runs_sha256: sha256_hex(lines.as_bytes()),
    })
}
