//! Calibration eligibility. A family's fixtures enter the calibration corpus only after that
//! family's target-impact study passed; each promoted copy carries the study metadata that
//! links it to its matched pair, its intervention verification and the impact report digest.
//! The acceptance-time fixtures are never modified.
use std::{fs, path::Path};

use anyhow::Context;

use super::store::{RunRecord, StudyDir};

fn link_tree(from: &Path, to: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            link_tree(&entry.path(), &target)?;
        } else if fs::hard_link(entry.path(), &target).is_err() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Adds the study provenance to a fixture's `[provenance]` table.
pub fn annotate(text: &str, extra: &[(&str, String)]) -> anyhow::Result<String> {
    let mut table: toml::Table = text.parse().context("fixture.toml")?;
    let provenance = table
        .get_mut("provenance")
        .and_then(toml::Value::as_table_mut)
        .context("fixture has no [provenance]")?;
    for (key, value) in extra {
        provenance.insert((*key).to_owned(), toml::Value::String(value.clone()));
    }
    Ok(toml::to_string(&table)?)
}

/// Copies one accepted capture into the calibration corpus with its study provenance.
pub fn promote(
    study: &StudyDir,
    study_id: &str,
    run: &RunRecord,
    group: &str,
    impact_sha256: &str,
) -> anyhow::Result<()> {
    let name = run
        .fixture
        .as_deref()
        .context("accepted run has no fixture")?;
    let from = study.fixtures().join(name);
    let to = study.calibration().join("fixtures").join(name);
    if to.exists() {
        return Ok(());
    }
    link_tree(&from, &to)?;
    // The linked copy of fixture.toml must be a distinct file before it is rewritten.
    let original = fs::read_to_string(from.join("fixture.toml"))?;
    fs::remove_file(to.join("fixture.toml"))?;
    let verification = run.metrics["supporting"].to_string();
    let annotated = annotate(
        &original,
        &[
            ("study", "diagnosis-v2".to_owned()),
            ("study_id", study_id.to_owned()),
            ("matched_group", group.to_owned()),
            ("intervention_verification", verification),
            ("target_impact_report_sha256", impact_sha256.to_owned()),
        ],
    )?;
    fs::write(to.join("fixture.toml"), annotated)?;
    Ok(())
}
