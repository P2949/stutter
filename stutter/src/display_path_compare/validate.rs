use super::DisplayPathExpectation;
use crate::{
    display_topology::{ConnectorInfo, DisplayTopologySnapshot},
    process_tree::TaskClass,
    report::ReportAnalysisJson,
};

pub(super) fn validate_display_path_expectation(
    expect: Option<DisplayPathExpectation>,
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    let Some(expect) = expect else {
        return;
    };
    match expect {
        DisplayPathExpectation::DirectToOffload => {
            if same_scanout_gpu(baseline, test) {
                warn(
                    warnings,
                    max_severity,
                    2,
                    "expected direct-to-offload but baseline and test scanout GPU did not differ",
                );
            }
            if test.display_path_diagnosis.is_cross_gpu != Some(true) {
                warn(
                    warnings,
                    max_severity,
                    2,
                    "expected direct-to-offload but test run was not identified as cross-GPU",
                );
            }
        }
        DisplayPathExpectation::OffloadToDirect => {
            if same_scanout_gpu(baseline, test) {
                warn(
                    warnings,
                    max_severity,
                    2,
                    "expected offload-to-direct but baseline and test scanout GPU did not differ",
                );
            }
            if baseline.display_path_diagnosis.is_cross_gpu != Some(true) {
                warn(
                    warnings,
                    max_severity,
                    2,
                    "expected offload-to-direct but baseline run was not identified as cross-GPU",
                );
            }
        }
        DisplayPathExpectation::Unknown => {}
    }
}

pub(super) fn validate_comparability(
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    validate_run_identity(baseline, test, warnings, max_severity);
    validate_frame_coverage(baseline, test, warnings, max_severity);
    validate_display_metadata(baseline, test, warnings, max_severity);
    if baseline.data_quality.level != crate::report::DataQualityLevel::High
        || test.data_quality.level != crate::report::DataQualityLevel::High
    {
        warn(
            warnings,
            max_severity,
            1,
            "one or both reports have non-high data quality",
        );
    }
}

pub(super) fn validate_topology_match(
    baseline: Option<&DisplayTopologySnapshot>,
    test: Option<&DisplayTopologySnapshot>,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    let Some(baseline) = baseline else {
        return;
    };
    let Some(test) = test else {
        return;
    };
    let baseline_connector = selected_connector(baseline);
    let test_connector = selected_connector(test);
    if baseline_connector
        .zip(test_connector)
        .is_some_and(|(baseline, test)| baseline.edid_hash != test.edid_hash)
    {
        warn(
            warnings,
            max_severity,
            1,
            "comparison downgraded: connected display EDID differs",
        );
    }
    if baseline_connector
        .zip(test_connector)
        .is_some_and(|(baseline, test)| baseline.modes.first() != test.modes.first())
    {
        warn(
            warnings,
            max_severity,
            1,
            "comparison downgraded: test and baseline used different refresh modes",
        );
    }
}

pub(super) fn validate_probe_match(
    label: &str,
    baseline_count: u64,
    test_count: u64,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    if (baseline_count == 0) != (test_count == 0) {
        warn(
            warnings,
            max_severity,
            1,
            format!("{label} availability differs between runs"),
        );
    }
}

fn validate_run_identity(
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    if baseline.session.core.run_name.is_some()
        && test.session.core.run_name.is_some()
        && baseline.session.core.run_name != test.session.core.run_name
    {
        warn(
            warnings,
            max_severity,
            2,
            "different scenario/run names; comparison may not isolate display path",
        );
    }
    if top_task_class(baseline) != top_task_class(test) {
        warn(
            warnings,
            max_severity,
            1,
            "top task class differs between runs",
        );
    }
    if top_process_comm(baseline) != top_process_comm(test) {
        warn(
            warnings,
            max_severity,
            1,
            "top process differs between runs",
        );
    }
}

fn validate_frame_coverage(
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    let baseline_duration = baseline.session.core.duration_ms.max(1) as f64;
    let test_duration = test.session.core.duration_ms.max(1) as f64;
    let duration_delta = ((test_duration - baseline_duration) / baseline_duration).abs();
    if duration_delta > 0.20 {
        warn(
            warnings,
            max_severity,
            2,
            "durations differ by more than 20%",
        );
    } else if duration_delta > 0.10 {
        warn(
            warnings,
            max_severity,
            1,
            "durations differ by more than 10%",
        );
    }
    if baseline.frame_pacing.frame_count == 0 || test.frame_pacing.frame_count == 0 {
        warn(
            warnings,
            max_severity,
            2,
            "one or both runs lack frame events",
        );
    }
    let frame_delta = rough_count_delta(
        baseline.frame_pacing.frame_count,
        test.frame_pacing.frame_count,
    );
    if frame_delta > 0.25 {
        warn(
            warnings,
            max_severity,
            2,
            "frame counts differ by more than 25%",
        );
    } else if frame_delta > 0.15 {
        warn(
            warnings,
            max_severity,
            1,
            "frame counts differ by more than 15%",
        );
    }
}

fn validate_display_metadata(
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
) {
    if display_session_type(baseline) != display_session_type(test) {
        warn(
            warnings,
            max_severity,
            1,
            "session type differs between runs",
        );
    }
    if display_compositor(baseline) != display_compositor(test) {
        warn(warnings, max_severity, 1, "compositor differs between runs");
    }
    if display_render_driver(baseline) != display_render_driver(test) {
        warn(
            warnings,
            max_severity,
            2,
            "comparison downgraded: render GPU changed",
        );
    }
    if display_connector(baseline) != display_connector(test) {
        warn(warnings, max_severity, 1, "connector differs between runs");
    }
}

fn warn(
    warnings: &mut Vec<String>,
    max_severity: &mut u8,
    severity: u8,
    message: impl Into<String>,
) {
    *max_severity = (*max_severity).max(severity);
    warnings.push(message.into());
}

fn same_scanout_gpu(baseline: &ReportAnalysisJson, test: &ReportAnalysisJson) -> bool {
    display_scanout_driver(baseline)
        .zip(display_scanout_driver(test))
        .is_some_and(|(baseline, test)| baseline == test)
        && display_scanout_card(baseline)
            .zip(display_scanout_card(test))
            .is_none_or(|(baseline, test)| baseline == test)
}

fn rough_count_delta(left: usize, right: usize) -> f64 {
    let base = left.max(1) as f64;
    ((right as f64 - left as f64) / base).abs()
}

fn top_process_comm(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .tasks
        .first()
        .map(|task| task.process_comm.as_str())
}

fn display_session_type(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.session_type.as_deref())
}

fn display_compositor(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.compositor.as_deref())
}

fn display_render_driver(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.render_driver.as_deref())
}

fn display_scanout_driver(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.scanout_driver.as_deref())
}

fn display_scanout_card(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.scanout_card.as_deref())
}

fn display_connector(analysis: &ReportAnalysisJson) -> Option<&str> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|path| path.connector.as_deref())
}

fn selected_connector(topology: &DisplayTopologySnapshot) -> Option<&ConnectorInfo> {
    let guess = topology.guessed_path.as_ref()?;
    let scanout_card = guess.scanout_card.as_deref()?;
    let connector_name = guess.connector.as_deref()?;
    topology
        .connectors
        .iter()
        .find(|connector| connector.card == scanout_card && connector.name == connector_name)
}

fn top_task_class(analysis: &ReportAnalysisJson) -> Option<TaskClass> {
    analysis.session.tasks.first().map(|task| task.class)
}
