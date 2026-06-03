use std::{
    fs::File,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::Deserialize;

use crate::advisor::{AdvisorFixPlan, models::AdvisorReport};

#[derive(Debug, Clone)]
pub(crate) struct ProveFixCommandInput {
    pub plan: PathBuf,
    pub profiles: PathBuf,
    pub tree_pid: u32,
    pub scenario_name: Option<String>,
    pub workload_label: Option<String>,
    pub route_label: Option<String>,
    pub duration_seconds: u64,
    pub baseline_runs: Option<usize>,
    pub test_runs: Option<usize>,
    pub baseline_profile: String,
    pub html: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct ProveFixWorkflow {
    pub plan_id: String,
    pub scenario_name: Option<String>,
    pub baseline_runs_required: usize,
    pub test_runs_required: usize,
    pub baseline_commands: Vec<String>,
    pub tune_command: String,
    pub validate_command: String,
}

pub(crate) fn prove_fix_command(input: ProveFixCommandInput) -> anyhow::Result<()> {
    let workflow = build_prove_fix_workflow(&input)?;
    print!("{}", render_prove_fix_workflow(&input, &workflow));
    Ok(())
}

pub(crate) fn build_prove_fix_workflow(
    input: &ProveFixCommandInput,
) -> anyhow::Result<ProveFixWorkflow> {
    let fix_plan = load_fix_plan(&input.plan)?;
    let scenario_name = normalize_scenario(
        input
            .scenario_name
            .clone()
            .or_else(|| fix_plan.validation.scenario_name.clone()),
    )?;
    let baseline_runs_required = input
        .baseline_runs
        .unwrap_or(fix_plan.validation.baseline_runs_required)
        .max(1);
    let test_runs_required = input
        .test_runs
        .unwrap_or(fix_plan.validation.test_runs_required)
        .max(1);
    let run_prefix = run_prefix(scenario_name.as_deref(), &fix_plan.id);
    let identity_args = identity_args(
        scenario_name.as_deref(),
        input.workload_label.as_deref(),
        input.route_label.as_deref(),
    );

    let baseline_commands = (1..=baseline_runs_required)
        .map(|index| {
            let mut args = vec![
                "stutter".to_owned(),
                "record".to_owned(),
                "--tree-pid".to_owned(),
                input.tree_pid.to_string(),
                "--duration".to_owned(),
                input.duration_seconds.to_string(),
                "--run-name".to_owned(),
                shell_quote_value(&format!("{run_prefix}-baseline-{index}")),
            ];
            args.extend(identity_args.clone());
            args.join(" ")
        })
        .collect::<Vec<_>>();

    let mut tune_args = vec![
        "stutter".to_owned(),
        "tune".to_owned(),
        "--tree-pid".to_owned(),
        input.tree_pid.to_string(),
        "--profiles".to_owned(),
        shell_quote_path(&input.profiles),
        "--runs".to_owned(),
        test_runs_required.to_string(),
        "--baseline-profile".to_owned(),
        shell_quote_value(&input.baseline_profile),
    ];
    tune_args.extend(identity_args);

    let mut validate_args = vec![
        "stutter".to_owned(),
        "recommend".to_owned(),
        "--fix-plan".to_owned(),
        shell_quote_path(&input.plan),
        "--baseline".to_owned(),
    ];
    validate_args.extend(
        (1..=baseline_runs_required)
            .map(|index| shell_quote_value(&format!("<{run_prefix}-baseline-{index}>"))),
    );
    validate_args.extend([
        "--tune".to_owned(),
        shell_quote_value("<tune-dir>"),
        "--html".to_owned(),
        shell_quote_path(&input.html),
    ]);

    Ok(ProveFixWorkflow {
        plan_id: fix_plan.id,
        scenario_name,
        baseline_runs_required,
        test_runs_required,
        baseline_commands,
        tune_command: tune_args.join(" "),
        validate_command: validate_args.join(" "),
    })
}

pub(crate) fn render_prove_fix_workflow(
    input: &ProveFixCommandInput,
    workflow: &ProveFixWorkflow,
) -> String {
    let mut out = String::new();
    pushln(&mut out, "Guided fix validation workflow");
    pushln(&mut out, format!("Plan: {}", input.plan.display()));
    pushln(&mut out, format!("Plan id: {}", workflow.plan_id));
    pushln(
        &mut out,
        format!(
            "Scenario: {}",
            workflow
                .scenario_name
                .as_deref()
                .unwrap_or("not set; pass --scenario to make comparability explicit")
        ),
    );
    pushln(
        &mut out,
        format!("Baseline runs: {}", workflow.baseline_runs_required),
    );
    pushln(
        &mut out,
        format!("Test runs: {}", workflow.test_runs_required),
    );
    pushln(&mut out, "");

    pushln(&mut out, "1. Record baseline:");
    for command in &workflow.baseline_commands {
        pushln(&mut out, format!("   {command}"));
    }
    pushln(&mut out, "");

    pushln(&mut out, "2. Run tuning:");
    pushln(&mut out, format!("   {}", workflow.tune_command));
    pushln(&mut out, "");

    pushln(&mut out, "3. Validate:");
    pushln(&mut out, format!("   {}", workflow.validate_command));
    pushln(&mut out, "");
    pushln(
        &mut out,
        "Keep the scenario, workload, and route labels identical across baseline and tuning runs.",
    );
    out
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FixPlanDocument {
    Plan(Box<AdvisorFixPlan>),
    Report(AdvisorReport),
}

fn load_fix_plan(path: &Path) -> anyhow::Result<AdvisorFixPlan> {
    let file =
        File::open(path).with_context(|| format!("failed to open fix plan {}", path.display()))?;
    let document: FixPlanDocument = serde_json::from_reader(file)
        .with_context(|| format!("failed to parse fix plan {}", path.display()))?;

    match document {
        FixPlanDocument::Plan(plan) => Ok(*plan),
        FixPlanDocument::Report(report) => report
            .fix_plans
            .into_iter()
            .next()
            .or_else(|| {
                report
                    .recommendations
                    .into_iter()
                    .filter_map(|recommendation| recommendation.fix_plan)
                    .next()
            })
            .with_context(|| format!("advisor report {} contains no fix plans", path.display())),
    }
}

fn normalize_scenario(value: Option<String>) -> anyhow::Result<Option<String>> {
    let normalized = crate::scenario::normalize_identity_label(value.as_deref());
    if let Some(scenario_name) = normalized.as_deref() {
        crate::scenario::validate_scenario_name(scenario_name)?;
    }
    Ok(normalized)
}

fn identity_args(
    scenario_name: Option<&str>,
    workload_label: Option<&str>,
    route_label: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    push_optional_arg(&mut args, "--scenario", scenario_name);
    push_optional_arg(&mut args, "--workload-label", workload_label);
    push_optional_arg(&mut args, "--route-label", route_label);
    args
}

fn push_optional_arg(args: &mut Vec<String>, name: &str, value: Option<&str>) {
    let Some(value) = crate::scenario::normalize_identity_label(value) else {
        return;
    };
    args.push(name.to_owned());
    args.push(shell_quote_value(&value));
}

fn run_prefix(scenario_name: Option<&str>, plan_id: &str) -> String {
    scenario_name
        .map(sanitize_run_label)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let suffix = plan_id
                .rsplit(':')
                .next()
                .map(sanitize_run_label)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "fix-validation".to_owned());
            format!("fix-{suffix}")
        })
}

fn sanitize_run_label(value: &str) -> String {
    let mut out = String::new();
    let mut previous_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            out.push('-');
            previous_dash = true;
        }
    }
    out.trim_matches('-').to_owned()
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote_value(&path.display().to_string())
}

fn shell_quote_value(value: &str) -> String {
    if value.is_empty() {
        return "''".to_owned();
    }
    if value.chars().all(is_shell_safe_char) {
        return value.to_owned();
    }

    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

fn is_shell_safe_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '/' | '.' | '_' | '-' | '+' | ':' | '=' | ',' | '@' | '<' | '>'
        )
}

fn pushln(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;
    use crate::diagnosis::StutterCause;

    #[test]
    fn prove_fix_uses_plan_defaults_and_identity_flags() {
        let tempdir = tempfile::tempdir().unwrap();
        let plan_path = tempdir.path().join("plan.json");
        let mut plan = crate::advisor::scheduler_profile_fix_plan(
            Path::new("/tmp/run"),
            StutterCause::GameThreadSchedulerDelay,
            Some(123),
            Some(Path::new("/tmp/profiles.toml")),
            None,
        );
        plan.validation.scenario_name = Some("city-run".to_owned());
        plan.validation.baseline_runs_required = 2;
        plan.validation.test_runs_required = 3;
        fs::write(&plan_path, serde_json::to_string_pretty(&plan).unwrap()).unwrap();

        let input = ProveFixCommandInput {
            plan: plan_path,
            profiles: PathBuf::from("/tmp/profiles.toml"),
            tree_pid: 123,
            scenario_name: None,
            workload_label: Some("Game.exe".to_owned()),
            route_label: Some("city-loop".to_owned()),
            duration_seconds: 180,
            baseline_runs: None,
            test_runs: None,
            baseline_profile: "baseline-online".to_owned(),
            html: PathBuf::from("/tmp/fix-validation.html"),
        };

        let workflow = build_prove_fix_workflow(&input).unwrap();
        let rendered = render_prove_fix_workflow(&input, &workflow);

        assert_eq!(workflow.baseline_runs_required, 2);
        assert_eq!(workflow.test_runs_required, 3);
        assert!(rendered.contains("stutter record --tree-pid 123 --duration 180"));
        assert!(rendered.contains("--scenario city-run"));
        assert!(rendered.contains("--workload-label Game.exe"));
        assert!(rendered.contains("--route-label city-loop"));
        assert!(
            rendered.contains("stutter tune --tree-pid 123 --profiles /tmp/profiles.toml --runs 3")
        );
        assert!(rendered.contains("stutter recommend --fix-plan"));
        assert!(rendered.contains("<city-run-baseline-1>"));
        assert!(rendered.contains("<city-run-baseline-2>"));
    }

    #[test]
    fn prove_fix_accepts_advisor_report_json() {
        let tempdir = tempfile::tempdir().unwrap();
        let report_path = tempdir.path().join("advisor.json");
        let plan = crate::advisor::scheduler_profile_fix_plan(
            Path::new("/tmp/run"),
            StutterCause::GameThreadSchedulerDelay,
            Some(123),
            Some(Path::new("/tmp/profiles.toml")),
            None,
        );
        let report = serde_json::json!({
            "schema_version": 2,
            "run": "/tmp/run",
            "data_quality": "High",
            "verdict": "TryProfileTuning",
            "recommendations": [],
            "fix_plans": [plan],
            "warnings": []
        });
        fs::write(&report_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();

        let loaded = load_fix_plan(&report_path).unwrap();

        assert_eq!(
            loaded.id,
            "advisor-fix:game-thread-scheduler-delay:cpu-affinity-profile"
        );
    }
}
