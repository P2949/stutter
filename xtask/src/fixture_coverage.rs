use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};

#[derive(Debug, Default)]
struct FixtureTomlSummary {
    name: String,
    source: String,
    sanitized_capture_id: Option<String>,
    gpu_vendor: Option<String>,
    compositor: Option<String>,
    scenario: Option<String>,
    kernel_version_bucket: Option<String>,
    data_quality: Option<String>,
    expected_behavior: Option<String>,
    privacy_fields: BTreeMap<String, bool>,
}

#[derive(Debug)]
pub struct FixtureCoverageReport {
    pub real_fixture_count: usize,
    pub synthetic_fixture_count: usize,
    pub vendors: BTreeMap<String, usize>,
    pub compositors: BTreeMap<String, usize>,
    pub scenarios: BTreeMap<String, usize>,
    pub kernels: BTreeMap<String, usize>,
    pub data_quality: BTreeMap<String, usize>,
    pub known_false_positive_count: usize,
    pub known_false_negative_count: usize,
    pub distinct_capture_ids: usize,
    pub missing_cells: Vec<String>,
    pub maturity_warnings: Vec<String>,
    pub privacy_warnings: Vec<String>,
}

const MIN_REAL_FIXTURES: usize = 20;
const MIN_FALSE_POSITIVE_FIXTURES: usize = 3;
const MIN_KNOWN_FALSE_NEGATIVE_FIXTURES: usize = 3;
const MIN_DISTINCT_CAPTURE_IDS: usize = 20;

pub fn run_fixture_coverage(root: &Path, html: Option<&Path>) -> anyhow::Result<()> {
    let report = fixture_coverage_report(root)?;
    print_fixture_coverage_report(&report);
    if let Some(path) = html {
        write_fixture_coverage_html(path, &report)?;
        println!("fixture_coverage_html={}", path.display());
    }
    if !report.missing_cells.is_empty() {
        bail!(
            "fixture coverage missing required cells: {}",
            report.missing_cells.join(", ")
        );
    }
    Ok(())
}

pub fn fixture_coverage_report(root: &Path) -> anyhow::Result<FixtureCoverageReport> {
    let fixtures_root = root.join("stutter/tests/fixtures/runs");
    let mut report = FixtureCoverageReport {
        real_fixture_count: 0,
        synthetic_fixture_count: 0,
        vendors: BTreeMap::new(),
        compositors: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        kernels: BTreeMap::new(),
        data_quality: BTreeMap::new(),
        known_false_positive_count: 0,
        known_false_negative_count: 0,
        distinct_capture_ids: 0,
        missing_cells: Vec::new(),
        maturity_warnings: Vec::new(),
        privacy_warnings: Vec::new(),
    };
    let mut capture_ids = BTreeSet::new();

    for path in fixture_toml_paths(&fixtures_root)? {
        let summary = parse_fixture_toml_summary(&path)?;
        let is_real = summary.name.starts_with("real_")
            || summary.source == "sanitized-real-recording"
            || summary.source == "validation-corpus";
        if is_real {
            report.real_fixture_count += 1;
            if let Some(capture_id) = summary.sanitized_capture_id.as_deref() {
                capture_ids.insert(capture_id.to_owned());
            } else {
                report.privacy_warnings.push(format!(
                    "{}: real fixture is missing platform.sanitized_capture_id",
                    path.display()
                ));
            }
            for field in [
                "titles_redacted",
                "paths_redacted",
                "hostnames_redacted",
                "usernames_redacted",
            ] {
                if summary.privacy_fields.get(field) != Some(&true) {
                    report.privacy_warnings.push(format!(
                        "{}: privacy.{field} must be true for real fixtures",
                        path.display()
                    ));
                }
            }
        } else {
            report.synthetic_fixture_count += 1;
        }
        if summary.expected_behavior.as_deref() == Some("known_miss") {
            report.known_false_negative_count += 1;
        }
        if let Some(scenario) = summary.scenario.as_deref()
            && scenario == "false-positive"
        {
            report.known_false_positive_count += 1;
        }
        increment_opt(&mut report.vendors, summary.gpu_vendor);
        increment_opt(&mut report.compositors, summary.compositor);
        increment_opt(&mut report.scenarios, summary.scenario);
        increment_opt(&mut report.kernels, summary.kernel_version_bucket);
        increment_opt(&mut report.data_quality, summary.data_quality);
    }
    report.distinct_capture_ids = capture_ids.len();

    for required in ["AMD", "NVIDIA", "Intel"] {
        push_missing(
            &mut report.missing_cells,
            "vendor",
            required,
            &report.vendors,
        );
    }
    for required in ["Sway", "Hyprland", "Gamescope", "KWin", "GNOME"] {
        push_missing(
            &mut report.missing_cells,
            "compositor",
            required,
            &report.compositors,
        );
    }
    for required in [
        "clean",
        "false-positive",
        "cpu-bound",
        "gpu-bound",
        "irq",
        "compositor",
    ] {
        push_missing(
            &mut report.missing_cells,
            "scenario",
            required,
            &report.scenarios,
        );
    }
    push_minimum_warning(
        &mut report.maturity_warnings,
        "real fixtures",
        report.real_fixture_count,
        MIN_REAL_FIXTURES,
    );
    push_minimum_warning(
        &mut report.maturity_warnings,
        "false-positive fixtures",
        report.known_false_positive_count,
        MIN_FALSE_POSITIVE_FIXTURES,
    );
    push_minimum_warning(
        &mut report.maturity_warnings,
        "known false-negative fixtures",
        report.known_false_negative_count,
        MIN_KNOWN_FALSE_NEGATIVE_FIXTURES,
    );
    push_minimum_warning(
        &mut report.maturity_warnings,
        "distinct sanitized capture ids",
        report.distinct_capture_ids,
        MIN_DISTINCT_CAPTURE_IDS,
    );

    Ok(report)
}

fn fixture_toml_paths(fixtures_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(fixtures_root)
        .with_context(|| format!("failed to read {}", fixtures_root.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let path = entry.path().join("fixture.toml");
            if path.is_file() {
                paths.push(path);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn parse_fixture_toml_summary(path: &Path) -> anyhow::Result<FixtureTomlSummary> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut summary = FixtureTomlSummary::default();
    let mut section = "";
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed;
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = unquote(value.trim());
        match (section, key) {
            ("", "name") => summary.name = value,
            ("", "source") => summary.source = value,
            ("[platform]", "gpu_vendor") => summary.gpu_vendor = Some(value),
            ("[platform]", "compositor") => summary.compositor = Some(value),
            ("[platform]", "scenario") => summary.scenario = Some(value),
            ("[platform]", "sanitized_capture_id") => {
                summary.sanitized_capture_id = Some(value);
            }
            ("[platform]", "kernel_version_bucket") => {
                summary.kernel_version_bucket = Some(value);
            }
            ("[expected]", "data_quality") => summary.data_quality = Some(value),
            ("[expected]", "expected_behavior") => summary.expected_behavior = Some(value),
            ("[privacy]", key) => {
                summary
                    .privacy_fields
                    .insert(key.to_owned(), matches!(value.as_str(), "true" | "True"));
            }
            _ => {}
        }
    }
    Ok(summary)
}

fn unquote(value: &str) -> String {
    value.trim_matches('"').trim_matches('\'').trim().to_owned()
}

fn increment_opt(map: &mut BTreeMap<String, usize>, key: Option<String>) {
    let Some(key) = key else {
        return;
    };
    if key.trim().is_empty() {
        return;
    }
    *map.entry(key).or_default() += 1;
}

fn push_missing(
    missing: &mut Vec<String>,
    label: &str,
    required: &str,
    map: &BTreeMap<String, usize>,
) {
    if !map.contains_key(required) {
        missing.push(format!("{label}:{required}"));
    }
}

fn push_minimum_warning(warnings: &mut Vec<String>, label: &str, actual: usize, required: usize) {
    if actual < required {
        warnings.push(format!(
            "{label}: {actual} present, target {required} for low-risk-stable maturity"
        ));
    }
}

fn print_fixture_coverage_report(report: &FixtureCoverageReport) {
    println!("fixture coverage");
    println!("real fixtures: {}", report.real_fixture_count);
    println!("synthetic fixtures: {}", report.synthetic_fixture_count);
    println!(
        "distinct sanitized capture ids: {}",
        report.distinct_capture_ids
    );
    println!("vendors: {}", format_counts(&report.vendors));
    println!("compositors: {}", format_counts(&report.compositors));
    println!("scenarios: {}", format_counts(&report.scenarios));
    println!("kernels: {}", format_counts(&report.kernels));
    println!("data quality: {}", format_counts(&report.data_quality));
    println!(
        "known false positives: {}",
        report.known_false_positive_count
    );
    println!(
        "known false negatives: {}",
        report.known_false_negative_count
    );
    if report.missing_cells.is_empty() {
        println!("missing: none");
    } else {
        println!("missing: {}", report.missing_cells.join(", "));
    }
    if report.maturity_warnings.is_empty() {
        println!("maturity warnings: none");
    } else {
        println!("maturity warnings: {}", report.maturity_warnings.join("; "));
    }
    if report.privacy_warnings.is_empty() {
        println!("privacy warnings: none");
    } else {
        println!("privacy warnings: {}", report.privacy_warnings.join("; "));
    }
}

fn format_counts(values: &BTreeMap<String, usize>) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }
    values
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn write_fixture_coverage_html(path: &Path, report: &FixtureCoverageReport) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, render_fixture_coverage_html(report))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn render_fixture_coverage_html(report: &FixtureCoverageReport) -> String {
    let maturity = if report.maturity_warnings.is_empty() {
        "<li>All warning-only maturity targets are met.</li>".to_owned()
    } else {
        list_items(&report.maturity_warnings)
    };
    let privacy = if report.privacy_warnings.is_empty() {
        "<li>No privacy manifest warnings.</li>".to_owned()
    } else {
        list_items(&report.privacy_warnings)
    };
    let missing = if report.missing_cells.is_empty() {
        "<li>No required coverage cells missing.</li>".to_owned()
    } else {
        list_items(&report.missing_cells)
    };

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Stutter Validation Corpus Coverage</title>
<style>
:root {{
  color-scheme: light dark;
  font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}}
body {{ margin: 0; background: #f8fafc; color: #172033; }}
main {{ max-width: 1100px; margin: 0 auto; padding: 32px 20px 48px; }}
h1 {{ margin: 0 0 20px; font-size: 1.8rem; }}
h2 {{ margin: 28px 0 12px; font-size: 1.1rem; }}
.summary {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 12px; }}
.metric {{ border: 1px solid #d8dee9; border-radius: 8px; padding: 12px; background: #fff; }}
.metric strong {{ display: block; font-size: 1.45rem; }}
table {{ border-collapse: collapse; width: 100%; background: #fff; border: 1px solid #d8dee9; }}
th, td {{ padding: 8px 10px; border-bottom: 1px solid #e5e9f0; text-align: left; }}
th {{ background: #eef3f8; }}
ul {{ margin-top: 0; }}
@media (prefers-color-scheme: dark) {{
  body {{ background: #10151f; color: #e7ecf5; }}
  .metric, table {{ background: #161d2a; border-color: #2b3443; }}
  th {{ background: #1e2838; }}
  th, td {{ border-bottom-color: #2b3443; }}
}}
</style>
</head>
<body>
<main>
<h1>Validation Corpus Coverage</h1>
<section class="summary">
<div class="metric"><span>Real fixtures</span><strong>{}</strong></div>
<div class="metric"><span>Synthetic fixtures</span><strong>{}</strong></div>
<div class="metric"><span>Distinct capture IDs</span><strong>{}</strong></div>
<div class="metric"><span>False positives</span><strong>{}</strong></div>
<div class="metric"><span>Known misses</span><strong>{}</strong></div>
</section>
<h2>Coverage Matrix</h2>
<table>
<tr><th>Dimension</th><th>Counts</th></tr>
<tr><td>GPU vendors</td><td>{}</td></tr>
<tr><td>Compositors</td><td>{}</td></tr>
<tr><td>Scenarios</td><td>{}</td></tr>
<tr><td>Kernel buckets</td><td>{}</td></tr>
<tr><td>Data quality</td><td>{}</td></tr>
</table>
<h2>Missing Required Cells</h2>
<ul>{missing}</ul>
<h2>Warning-Only Maturity Targets</h2>
<ul>{maturity}</ul>
<h2>Privacy Manifest Checks</h2>
<ul>{privacy}</ul>
</main>
</body>
</html>
"#,
        report.real_fixture_count,
        report.synthetic_fixture_count,
        report.distinct_capture_ids,
        report.known_false_positive_count,
        report.known_false_negative_count,
        escape_html(&format_counts(&report.vendors)),
        escape_html(&format_counts(&report.compositors)),
        escape_html(&format_counts(&report.scenarios)),
        escape_html(&format_counts(&report.kernels)),
        escape_html(&format_counts(&report.data_quality)),
    )
}

fn list_items(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("<li>{}</li>", escape_html(item)))
        .collect::<Vec<_>>()
        .join("")
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
