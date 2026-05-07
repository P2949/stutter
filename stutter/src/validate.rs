use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    report::{self, DataQualityLevel, DataQualitySummary},
    session_io::{self, RunValidationReport},
};

#[derive(Debug, Clone)]
pub struct ValidateCommandInput {
    pub path: PathBuf,
    pub json: bool,
    pub strict: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidateCommandOutput {
    pub passed: bool,
    pub strict: bool,
    pub path: PathBuf,
    pub validation: RunValidationReport,
    pub data_quality: Option<DataQualitySummary>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn validate_command(input: ValidateCommandInput) -> anyhow::Result<()> {
    let output = validate_run_for_command(&input.path, input.strict);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_validate_output(&output));
    }

    if !output.passed {
        anyhow::bail!("validation failed for {}", input.path.display());
    }

    Ok(())
}

pub fn validate_run_for_command(path: &Path, strict: bool) -> ValidateCommandOutput {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let validation = match session_io::validate_run_dir(path) {
        Ok(report) => report,
        Err(err) => {
            push_unique(&mut errors, format!("artifact validation failed: {err:#}"));
            RunValidationReport {
                run_dir: path.to_path_buf(),
                ..Default::default()
            }
        }
    };

    extend_unique(&mut errors, validation.errors.iter().cloned());
    extend_unique(&mut warnings, validation.warnings.iter().cloned());

    let data_quality = match report::build_report_analysis(path, 10, 5, None) {
        Ok(analysis) => Some(analysis.data_quality),
        Err(err) => {
            push_unique(&mut errors, format!("analysis validation failed: {err:#}"));
            None
        }
    };

    if let Some(data_quality) = &data_quality {
        extend_unique(&mut errors, data_quality.validation_errors.iter().cloned());
        extend_unique(
            &mut warnings,
            data_quality.validation_warnings.iter().cloned(),
        );

        if matches!(data_quality.level, DataQualityLevel::Low) {
            push_unique(&mut errors, "data quality is Low".to_owned());
        }

        if strict {
            if !matches!(data_quality.level, DataQualityLevel::High) {
                push_unique(
                    &mut errors,
                    format!(
                        "strict mode requires High data quality, got {:?}",
                        data_quality.level
                    ),
                );
            }
            if data_quality.event_stream_write_errors > 0 {
                push_unique(
                    &mut errors,
                    format!(
                        "strict mode rejects event stream write errors: {}",
                        data_quality.event_stream_write_errors
                    ),
                );
            }
            if data_quality.spike_events_truncated {
                push_unique(
                    &mut errors,
                    "strict mode rejects truncated spike events".to_owned(),
                );
            }
            if data_quality.drop_counters_nonzero {
                push_unique(
                    &mut errors,
                    "strict mode rejects nonzero eBPF drop counters".to_owned(),
                );
            }
            if !data_quality.missing_optional_files.is_empty() {
                push_unique(
                    &mut errors,
                    format!(
                        "strict mode rejects missing optional files: {:?}",
                        data_quality.missing_optional_files
                    ),
                );
            }
        }
    }

    if strict && !warnings.is_empty() {
        push_unique(
            &mut errors,
            "strict mode rejects validation warnings".to_owned(),
        );
    }

    let passed = errors.is_empty();

    ValidateCommandOutput {
        passed,
        strict,
        path: path.to_path_buf(),
        validation,
        data_quality,
        errors,
        warnings,
    }
}

pub fn render_validate_output(output: &ValidateCommandOutput) -> String {
    let mut rendered = String::new();
    pushln(&mut rendered, "stutter validate");
    pushln(&mut rendered, "================");
    pushln(&mut rendered, format!("path: {}", output.path.display()));
    pushln(
        &mut rendered,
        format!(
            "result: {}",
            if output.passed { "passed" } else { "failed" }
        ),
    );
    pushln(&mut rendered, format!("strict: {}", output.strict));
    pushln(
        &mut rendered,
        format!(
            "data_quality: {}",
            output
                .data_quality
                .as_ref()
                .map(|data_quality| format!("{:?}", data_quality.level))
                .unwrap_or_else(|| "unavailable".to_owned())
        ),
    );
    pushln(
        &mut rendered,
        format!(
            "schema_version: {}",
            output
                .data_quality
                .as_ref()
                .map(|data_quality| data_quality.schema_version.to_string())
                .unwrap_or_else(|| "unavailable".to_owned())
        ),
    );
    pushln(
        &mut rendered,
        format!(
            "expected_schema_version: {}",
            output
                .data_quality
                .as_ref()
                .map(|data_quality| data_quality.expected_schema_version.to_string())
                .unwrap_or_else(|| "unavailable".to_owned())
        ),
    );
    pushln(&mut rendered, "");

    push_section(&mut rendered, "errors", &output.errors);
    push_section(&mut rendered, "warnings", &output.warnings);
    push_section(
        &mut rendered,
        "missing optional files",
        &output.validation.missing_optional_files,
    );

    rendered
}

fn push_section(output: &mut String, title: &str, values: &[String]) {
    pushln(output, title);
    pushln(output, "-".repeat(title.len()));
    if values.is_empty() {
        pushln(output, "none");
    } else {
        for value in values {
            pushln(output, value);
        }
    }
    pushln(output, "");
}

fn pushln(output: &mut String, line: impl AsRef<str>) {
    output.push_str(line.as_ref());
    output.push('\n');
}

fn extend_unique(values: &mut Vec<String>, new_values: impl IntoIterator<Item = String>) {
    for value in new_values {
        push_unique(values, value);
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use serde_json::Value;

    use super::*;
    use crate::test_fixture_builder;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-validate-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_corpus(name: &str) -> PathBuf {
        let root = temp_dir(name);
        test_fixture_builder::write_validation_corpus(&root).unwrap();
        root
    }

    #[test]
    fn validate_clean_run_passes() {
        let root = write_corpus("clean");
        let output = validate_run_for_command(&root.join("clean_run"), false);

        assert!(output.passed, "errors={:?}", output.errors);
        assert!(matches!(
            output.data_quality.as_ref().map(|dq| dq.level),
            Some(DataQualityLevel::High)
        ));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn validate_low_quality_fails() {
        let dir = temp_dir("missing-session");
        let output = validate_run_for_command(&dir, false);

        assert!(!output.passed);
        assert!(
            output
                .errors
                .iter()
                .any(|error| error.contains("missing mandatory session.json"))
                || output
                    .errors
                    .iter()
                    .any(|error| error.contains("analysis validation failed"))
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn validate_medium_quality_passes_without_strict() {
        let root = write_corpus("medium-default");
        let output = validate_run_for_command(&root.join("old_schema_warning"), false);

        assert!(output.passed, "errors={:?}", output.errors);
        assert!(matches!(
            output.data_quality.as_ref().map(|dq| dq.level),
            Some(DataQualityLevel::Medium)
        ));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn validate_medium_quality_fails_with_strict() {
        let root = write_corpus("medium-strict");
        let output = validate_run_for_command(&root.join("old_schema_warning"), true);

        assert!(!output.passed);
        assert!(
            output
                .errors
                .iter()
                .any(|error| error.contains("strict mode requires High data quality"))
        );

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn validate_json_output_is_structured() {
        let root = write_corpus("json-structured");
        let output = validate_run_for_command(&root.join("clean_run"), false);
        let value = serde_json::to_value(&output).unwrap();
        let object = value.as_object().unwrap();

        for key in [
            "passed",
            "strict",
            "path",
            "validation",
            "data_quality",
            "errors",
            "warnings",
        ] {
            assert!(object.contains_key(key), "missing key {key}: {value:?}");
        }
        assert!(matches!(object.get("passed"), Some(Value::Bool(true))));

        fs::remove_dir_all(root).ok();
    }
}
