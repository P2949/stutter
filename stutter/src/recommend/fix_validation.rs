use super::model::{
    BaselineTuneRecommendation, FixValidationBlocker, FixValidationMetricResult,
    FixValidationReport,
};
use crate::advisor::{AdvisorExpectedMetricMovement, AdvisorFixPlan, AdvisorFixValidationStatus};

pub fn validate_fix_plan_against_recommendation(
    plan: &AdvisorFixPlan,
    rec: &BaselineTuneRecommendation,
) -> FixValidationReport {
    let mut passed_criteria = Vec::new();
    let mut failed_criteria = Vec::new();
    let mut warnings = Vec::new();
    let mut metric_results = Vec::new();
    let blockers = fix_validation_blockers(rec);

    for criterion in &plan.validation.acceptance_criteria {
        let Some(metric) = rec
            .formal_metrics
            .iter()
            .find(|metric| metric.metric == criterion.metric)
        else {
            let message = format!("{}: required metric is missing", criterion.metric);
            failed_criteria.push(message.clone());
            warnings.push(message.clone());
            metric_results.push(FixValidationMetricResult {
                metric: criterion.metric.clone(),
                expected: expected_movement_label(criterion),
                actual: "missing".to_owned(),
                passed: false,
            });
            continue;
        };

        let actual_delta = if criterion.lower_is_better {
            metric.baseline_median - metric.tuned_median
        } else {
            metric.tuned_median - metric.baseline_median
        };
        let relative_improvement_percent = if metric.baseline_median.abs() > f64::EPSILON {
            actual_delta / metric.baseline_median.abs() * 100.0
        } else {
            0.0
        };
        let actual = format!(
            "actual improvement {relative_improvement_percent:.1}% (delta {actual_delta:.3}{})",
            metric.unit
        );
        let mut criterion_passed = true;

        if !metric.enough_samples {
            criterion_passed = false;
            failed_criteria.push(format!(
                "{}: not enough samples for fix validation",
                metric.metric
            ));
        }

        if criterion.required_ci_excludes_zero {
            match &metric.bootstrap_ci95 {
                Some(ci) if ci.lower > 0.0 => {}
                Some(ci) if ci.upper < 0.0 => {
                    criterion_passed = false;
                    failed_criteria.push(format!(
                        "{}: CI excludes zero in the wrong direction [{:.3}, {:.3}]",
                        metric.metric, ci.lower, ci.upper
                    ));
                }
                Some(ci) => {
                    criterion_passed = false;
                    failed_criteria.push(format!(
                        "{}: CI crosses zero [{:.3}, {:.3}]",
                        metric.metric, ci.lower, ci.upper
                    ));
                }
                None => {
                    criterion_passed = false;
                    failed_criteria.push(format!("{}: required CI is missing", metric.metric));
                }
            }
        }

        if let Some(minimum) = criterion.minimum_relative_improvement_percent {
            if actual_delta <= 0.0 {
                criterion_passed = false;
                failed_criteria.push(format!("{}: metric regressed", metric.metric));
            } else if relative_improvement_percent + f64::EPSILON < minimum {
                criterion_passed = false;
                failed_criteria.push(format!(
                    "{}: improvement {:.1}% is below required {:.1}%",
                    metric.metric, relative_improvement_percent, minimum
                ));
            }
        }

        if let Some(max_regression) = criterion.maximum_allowed_regression_percent
            && relative_improvement_percent < -max_regression
        {
            criterion_passed = false;
            failed_criteria.push(format!(
                "{}: regression {:.1}% exceeds allowed {:.1}%",
                metric.metric,
                relative_improvement_percent.abs(),
                max_regression
            ));
        }

        if criterion_passed {
            passed_criteria.push(format!(
                "{}: {}",
                metric.metric,
                expected_movement_label(criterion)
            ));
        }
        metric_results.push(FixValidationMetricResult {
            metric: metric.metric.clone(),
            expected: expected_movement_label(criterion),
            actual,
            passed: criterion_passed,
        });
    }

    let has_sample_warning = rec.warnings.iter().any(|warning| {
        warning.contains("not enough samples")
            || warning.contains("low sample count")
            || warning.contains("underpowered")
    });
    if has_sample_warning {
        warnings.push("formal A/B evidence is underpowered; do not count this as proof".to_owned());
    }
    for warning in &rec.warnings {
        if warning.contains("comparability") {
            warnings.push(warning.clone());
        }
    }

    let status = if !blockers.is_empty() {
        AdvisorFixValidationStatus::InvalidExperiment
    } else if failed_criteria.iter().any(|failure| {
        failure.contains("metric regressed")
            || failure.contains("wrong direction")
            || failure.contains("regression")
    }) {
        AdvisorFixValidationStatus::Rejected
    } else if failed_criteria.iter().any(|failure| {
        failure.contains("not enough samples") || failure.contains("required CI is missing")
    }) || has_sample_warning
    {
        AdvisorFixValidationStatus::Underpowered
    } else if failed_criteria
        .iter()
        .any(|failure| failure.contains("CI crosses zero"))
    {
        AdvisorFixValidationStatus::Inconclusive
    } else if failed_criteria.is_empty() {
        AdvisorFixValidationStatus::Validated
    } else {
        AdvisorFixValidationStatus::Rejected
    };

    let next_steps = match status {
        AdvisorFixValidationStatus::Validated => {
            vec!["Review the validated fix and keep rollback available before applying.".to_owned()]
        }
        AdvisorFixValidationStatus::Rejected => {
            vec!["Do not apply this fix; the A/B evidence rejected the hypothesis.".to_owned()]
        }
        AdvisorFixValidationStatus::Underpowered => vec![
            "Collect more baseline and tune runs under the same scenario before deciding."
                .to_owned(),
        ],
        AdvisorFixValidationStatus::InvalidExperiment => vec![
            "Repeat the experiment with comparable workload, frame coverage, and drop counters."
                .to_owned(),
        ],
        AdvisorFixValidationStatus::Inconclusive => {
            vec!["Repeat the experiment; current CIs do not prove or reject the fix.".to_owned()]
        }
        AdvisorFixValidationStatus::UnsafeToRun | AdvisorFixValidationStatus::NotRun => {
            vec!["Do not apply this fix from the current evidence.".to_owned()]
        }
    };

    FixValidationReport {
        schema_version: 1,
        fix_plan: plan.clone(),
        baseline_tune_recommendation: rec.clone(),
        status,
        passed_criteria,
        failed_criteria,
        warnings,
        blockers,
        next_steps,
        metric_results,
    }
}

fn expected_movement_label(criterion: &AdvisorExpectedMetricMovement) -> String {
    let mut parts = Vec::new();
    if let Some(percent) = criterion.minimum_relative_improvement_percent {
        let direction = if criterion.lower_is_better {
            "lower"
        } else {
            "higher"
        };
        parts.push(format!(">= {percent:.0}% {direction}"));
    }
    if let Some(percent) = criterion.maximum_allowed_regression_percent {
        parts.push(format!("no >{percent:.0}% regression"));
    }
    if criterion.required_ci_excludes_zero {
        parts.push("CI excludes zero".to_owned());
    }
    if parts.is_empty() {
        "observe and compare".to_owned()
    } else {
        parts.join(", ")
    }
}

fn fix_validation_blockers(rec: &BaselineTuneRecommendation) -> Vec<FixValidationBlocker> {
    let mut blockers = Vec::new();
    for warning in &rec.warnings {
        if warning.contains("drop-counters-nonzero") {
            push_blocker(&mut blockers, FixValidationBlocker::DropCountersNonzero);
        }
        if warning.contains("scenario-mismatch") {
            push_blocker(&mut blockers, FixValidationBlocker::ScenarioMismatch);
        }
        if warning.contains("missing-frame-data") || warning.contains("frame-count-mismatch") {
            push_blocker(&mut blockers, FixValidationBlocker::FrameCountMismatch);
        }
        if warning.contains("scored-sample-count-mismatch") {
            push_blocker(&mut blockers, FixValidationBlocker::CoverageTooLow);
        }
        if warning.contains("removed-task-count-mismatch")
            || warning.contains("task-identity")
            || warning.contains("thread-topology")
        {
            push_blocker(
                &mut blockers,
                FixValidationBlocker::MajorThreadTopologyShift,
            );
        }
    }
    blockers
}

fn push_blocker(blockers: &mut Vec<FixValidationBlocker>, blocker: FixValidationBlocker) {
    if !blockers.contains(&blocker) {
        blockers.push(blocker);
    }
}
