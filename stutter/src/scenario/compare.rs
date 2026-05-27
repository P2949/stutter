use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use super::{io::*, model::*};

pub struct ScenarioCompareInput {
    pub name: String,
    pub baseline: Option<PathBuf>,
    pub current: Option<PathBuf>,
    pub top: usize,
    pub json_summary: bool,
    pub validate: bool,
}

pub fn compare_scenario(input: ScenarioCompareInput) -> Result<()> {
    let scenario = load_scenario(&input.name)?;
    let state_dir = default_scenario_state_dir(&input.name)?;
    let index_path = state_dir.join("runs.json");

    let index: ScenarioRunsIndex = if index_path.exists() {
        let data = fs::read_to_string(&index_path)?;
        serde_json::from_str(&data)?
    } else {
        ScenarioRunsIndex::default()
    };

    let baseline_path = if let Some(p) = input.baseline {
        p
    } else {
        index.runs.iter()
            .rfind(|r| r.role == ScenarioRole::Baseline)
            .map(|r| r.run_dir.clone())
            .ok_or_else(|| anyhow::anyhow!("no baseline run found for scenario {}; run `stutter scenario run {} --role baseline` first", input.name, input.name))?
    };

    let current_path = if let Some(p) = input.current {
        p
    } else {
        index.runs.iter()
            .rfind(|r| r.role == ScenarioRole::Current)
            .map(|r| r.run_dir.clone())
            .ok_or_else(|| anyhow::anyhow!("no current run found for scenario {}; run `stutter scenario run {} --role current` first", input.name, input.name))?
    };

    if input.validate {
        if !crate::validate::validate_run_for_command(&baseline_path, false).passed {
            anyhow::bail!(
                "baseline run validation failed for {}",
                baseline_path.display()
            );
        }
        if !crate::validate::validate_run_for_command(&current_path, false).passed {
            anyhow::bail!(
                "current run validation failed for {}",
                current_path.display()
            );
        }
    }

    let baseline_missing = missing_expected_classes(&baseline_path, &scenario.expected_classes)?;
    let current_missing = missing_expected_classes(&current_path, &scenario.expected_classes)?;

    if input.json_summary {
        let diff =
            crate::report::diff::build_run_diff_summary(&baseline_path, &current_path, None)?
                .limited(input.top);
        let output = ScenarioCompareJson {
            scenario: scenario.name,
            baseline: baseline_path,
            current: current_path,
            diff,
            expected_class_check: ExpectedClassCheck {
                baseline_missing,
                current_missing,
            },
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("stutter scenario compare");
        println!("========================");
        println!("scenario: {}", scenario.name);
        if let Some(notes) = &scenario.notes {
            println!("notes: {}", notes);
        }
        println!("baseline: {}", baseline_path.display());
        println!("current:  {}", current_path.display());
        println!();

        for class in &baseline_missing {
            println!("WARNING: baseline missing expected class {}", class);
        }
        for class in &current_missing {
            println!("WARNING: current missing expected class {}", class);
        }
        if !baseline_missing.is_empty() || !current_missing.is_empty() {
            println!();
        }

        crate::report::print_diff_report(&baseline_path, &current_path, input.top, None)?;
    }

    Ok(())
}

pub fn missing_expected_classes(run_dir: &Path, expected: &[String]) -> Result<Vec<String>> {
    let session = crate::session_io::load_session(run_dir)?;
    let mut present = BTreeSet::new();
    for task in &session.tasks {
        present.insert(format!("{:?}", task.class));
    }

    let mut missing = Vec::new();
    for class in expected {
        if !present.contains(class) {
            missing.push(class.clone());
        }
    }
    Ok(missing)
}
