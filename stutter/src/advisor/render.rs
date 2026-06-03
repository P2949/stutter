use super::{
    fix_plan::{AdvisorExpectedMetricMovement, AdvisorFixPlan},
    models::AdvisorReport,
};

pub fn render_advisor_report(report: &AdvisorReport) -> String {
    let mut out = String::new();
    pushln(&mut out, "# stutter advisor");
    pushln(&mut out, "");
    pushln(&mut out, format!("Run: {}", report.run.display()));
    pushln(&mut out, format!("Data quality: {:?}", report.data_quality));
    pushln(&mut out, format!("Verdict: {:?}", report.verdict));
    pushln(&mut out, "");
    pushln(&mut out, "## Recommendations");
    pushln(&mut out, "");
    for recommendation in &report.recommendations {
        pushln(&mut out, format!("- {}", recommendation.title));
        pushln(
            &mut out,
            format!("  rationale: {}", recommendation.rationale),
        );
        pushln(
            &mut out,
            format!("  confidence: {:?}", recommendation.confidence),
        );
        pushln(
            &mut out,
            format!("  safety: {}", recommendation.safety_note),
        );
        pushln(
            &mut out,
            format!(
                "  safety class: {:?}; effect scope: {:?}; rollback: {:?}",
                recommendation.safety_risk.safety_class,
                recommendation.safety_risk.effect_scope,
                recommendation.safety_risk.rollback_requirement
            ),
        );
        for command in &recommendation.suggested_commands {
            pushln(&mut out, format!("  command: {command}"));
        }
        if let Some(fix_plan) = &recommendation.fix_plan {
            push_fix_plan(&mut out, fix_plan);
        }
    }
    if report.recommendations.is_empty() {
        pushln(&mut out, "- none");
    }
    pushln(&mut out, "");
    pushln(&mut out, "## Warnings");
    pushln(&mut out, "");
    if report.warnings.is_empty() {
        pushln(&mut out, "- none");
    } else {
        for warning in &report.warnings {
            pushln(&mut out, format!("- {warning}"));
        }
    }
    out
}

fn push_fix_plan(out: &mut String, fix_plan: &AdvisorFixPlan) {
    pushln(out, "");
    pushln(out, "### Fix hypothesis");
    pushln(out, "");
    pushln(out, format!("- kind: {}", fix_plan.kind.as_str()));
    pushln(out, format!("- cause: {:?}", fix_plan.cause));
    pushln(out, format!("- safety: {:?}", fix_plan.safety_class));
    pushln(out, format!("- effect scope: {:?}", fix_plan.effect_scope));
    pushln(out, format!("- rollback: {:?}", fix_plan.rollback));
    pushln(
        out,
        format!(
            "- allowed by default policy: {}",
            fix_plan.safety_risk.allowed_by_default_policy
        ),
    );
    pushln(
        out,
        format!(
            "- required policy mode: {}",
            fix_plan.safety_risk.required_policy_mode
        ),
    );
    if let Some(path) = &fix_plan.candidate_plan_path {
        pushln(out, format!("- candidate plan path: {}", path.display()));
    }
    pushln(out, "");
    pushln(out, "Expected metric movement:");
    pushln(out, "");
    if fix_plan.expected_metric_movement.is_empty() {
        pushln(out, "- investigation only; no applyable metric target yet");
    } else {
        pushln(out, "| Metric | Target |");
        pushln(out, "|---|---|");
        for movement in &fix_plan.expected_metric_movement {
            pushln(
                out,
                format!("| {} | {} |", movement.metric, metric_target(movement)),
            );
        }
    }
    pushln(out, "");
    pushln(out, "Validation:");
    pushln(out, "");
    pushln(
        out,
        format!(
            "1. Collect {} baseline runs.",
            fix_plan.validation.baseline_runs_required
        ),
    );
    pushln(
        out,
        format!(
            "2. Run the suggested experiment for {} runs.",
            fix_plan.validation.test_runs_required
        ),
    );
    pushln(
        out,
        format!("3. Compare with `{}`.", fix_plan.validation.compare_command),
    );
    pushln(
        out,
        "4. Accept only if the required CI excludes zero and guardrail metrics do not regress.",
    );
    pushln(out, "");
    pushln(out, "Acceptance criteria:");
    for criterion in &fix_plan.validation.acceptance_criteria {
        pushln(
            out,
            format!("- {}: {}", criterion.metric, metric_target(criterion)),
        );
    }
    pushln(out, "");
    pushln(out, "Stop conditions:");
    for condition in &fix_plan.validation.stop_conditions {
        pushln(out, format!("- {condition}"));
    }
}

fn metric_target(movement: &AdvisorExpectedMetricMovement) -> String {
    let mut parts = Vec::new();
    if let Some(percent) = movement.minimum_relative_improvement_percent {
        let direction = if movement.lower_is_better {
            "lower"
        } else {
            "higher"
        };
        parts.push(format!(">= {percent:.0}% {direction}"));
    }
    if let Some(percent) = movement.maximum_allowed_regression_percent {
        parts.push(format!("no >{percent:.0}% regression"));
    }
    if movement.required_ci_excludes_zero {
        parts.push("CI excludes zero".to_owned());
    }
    if parts.is_empty() {
        "observe and compare; no numeric pass threshold".to_owned()
    } else {
        parts.join(", ")
    }
}

fn pushln(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}
