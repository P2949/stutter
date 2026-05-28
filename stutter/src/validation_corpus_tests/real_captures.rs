use super::{assertions::*, fixture::*};
use crate::{diagnosis::StutterCause, report::DataQualityLevel};

#[test]
fn validation_corpus_real_clean_baseline() {
    let analysis = assert_fixture_from_metadata("real_clean_baseline");

    assert!(analysis.data_quality.validation_errors.is_empty());
    assert!(analysis.data_quality.validation_warnings.is_empty());
    assert!(
        analysis.cluster_analysis.clusters.is_empty()
            || no_primary_non_unknown_diagnosis(&analysis),
        "real_clean_baseline must not produce a non-Unknown primary diagnosis: {:?}",
        analysis
            .cluster_analysis
            .clusters
            .iter()
            .filter_map(|cluster| cluster.diagnosis.as_ref())
            .map(|diagnosis| &diagnosis.cause)
            .collect::<Vec<_>>()
    );
}

#[test]
fn validation_corpus_real_game_thread_scheduler_delay() {
    let analysis = assert_fixture_from_metadata("real_game_thread_scheduler_delay");

    assert_primary_anchor_class_in(
        &analysis,
        StutterCause::GameThreadSchedulerDelay,
        &[
            crate::process_tree::TaskClass::Game,
            crate::process_tree::TaskClass::GameRenderThread,
            crate::process_tree::TaskClass::GameWorkerThread,
            crate::process_tree::TaskClass::GameHelper,
            crate::process_tree::TaskClass::WineServer,
        ],
    );

    assert!(
        analysis.artifacts_summary.spike_count >= 3,
        "real_game_thread_scheduler_delay should contain clustered game/render/main-thread spikes"
    );
    assert!(
        analysis.artifacts_summary.frame_event_count >= 1,
        "real_game_thread_scheduler_delay should contain frame-correlation data near the scheduler spike"
    );
    assert!(
        analysis.artifacts_summary.interval_record_count >= 1,
        "real_game_thread_scheduler_delay should contain interval data so CPU pressure can be ruled out"
    );
    assert_eq!(
        analysis.artifacts_summary.irq_event_count, 0,
        "IRQ evidence should not dominate real_game_thread_scheduler_delay"
    );
    assert_eq!(
        analysis.artifacts_summary.block_io_event_count, 0,
        "block I/O evidence should not dominate real_game_thread_scheduler_delay"
    );
}

#[test]
fn validation_corpus_real_compositor_scheduler_delay() {
    let analysis = assert_fixture_from_metadata("real_compositor_scheduler_delay");

    let diagnosis = primary_diagnosis(&analysis)
        .expect("real_compositor_scheduler_delay expected a primary diagnosis");
    let evidence_text = diagnosis.evidence.join("\n").to_ascii_lowercase();
    assert!(
        evidence_text.contains("compositor thread") || evidence_text.contains("gamescope"),
        "real_compositor_scheduler_delay missing compositor/gamescope evidence; evidence was:\n{}",
        diagnosis.evidence.join("\n")
    );

    assert_primary_anchor_class_in(
        &analysis,
        StutterCause::CompositorSchedulerDelay,
        &[
            crate::process_tree::TaskClass::Compositor,
            crate::process_tree::TaskClass::GameScope,
        ],
    );

    assert!(
        analysis.artifacts_summary.frame_event_count >= 1,
        "real_compositor_scheduler_delay must contain frame data near the scheduler spike"
    );
    assert_eq!(
        analysis.artifacts_summary.irq_event_count, 0,
        "IRQ evidence should not dominate real_compositor_scheduler_delay"
    );
    assert_eq!(
        analysis.artifacts_summary.block_io_event_count, 0,
        "block I/O evidence should not dominate real_compositor_scheduler_delay"
    );
}

#[test]
fn validation_corpus_real_irq_overlap() {
    let analysis = assert_fixture_from_metadata("real_irq_overlap");

    assert!(
        matches!(
            analysis.data_quality.level,
            DataQualityLevel::High | DataQualityLevel::Medium
        ),
        "real_irq_overlap data quality should be High or Medium, got {:?}",
        analysis.data_quality.level
    );
    assert!(analysis.data_quality.validation_errors.is_empty());
    assert!(
        analysis.artifacts_summary.irq_event_count > 0,
        "real_irq_overlap must contain IRQ artifacts"
    );
    assert!(
        analysis.artifacts_summary.irq_event_count >= 4,
        "real_irq_overlap should include multiple IRQ events, including unrelated noise outside the spike window"
    );
    assert_eq!(
        analysis.artifacts_summary.block_io_event_count, 0,
        "block I/O evidence should not dominate real_irq_overlap"
    );
    assert_eq!(
        analysis.artifacts_summary.gpu_sample_count, 0,
        "GPU evidence should not dominate real_irq_overlap"
    );

    let candidate = find_candidate(&analysis, StutterCause::IrqDelayCandidate)
        .expect("real_irq_overlap missing IRQ diagnosis candidate");
    let candidate_evidence = candidate
        .evidence
        .iter()
        .map(|item| item.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        candidate_evidence.contains("IRQ"),
        "real_irq_overlap IRQ candidate evidence did not mention IRQ; evidence was:\n{}",
        candidate_evidence
    );
    assert!(
        !candidate_evidence.contains("147") && !candidate_evidence.contains("148"),
        "real_irq_overlap IRQ candidate evidence should stay focused on the correlated IRQ window and not report unrelated IRQ 147/148 noise; evidence was:\n{}",
        candidate_evidence
    );
}

#[test]
fn validation_corpus_real_gpu_bound_looking() {
    let analysis = assert_fixture_from_metadata("real_gpu_bound_looking");

    assert!(
        analysis.artifacts_summary.gpu_sample_count > 0,
        "real_gpu_bound_looking must contain GPU samples"
    );
    assert!(
        analysis.artifacts_summary.frame_event_count > 0,
        "real_gpu_bound_looking must contain frame events"
    );

    assert_candidate_contains(&analysis, StutterCause::GpuBoundCandidate, &["GPU busy"]);
}

#[test]
fn validation_corpus_real_block_io_overlap() {
    let analysis = assert_fixture_from_metadata("real_block_io_overlap");

    assert!(
        matches!(
            analysis.data_quality.level,
            DataQualityLevel::High | DataQualityLevel::Medium
        ),
        "real_block_io_overlap data quality should be High or Medium, got {:?}",
        analysis.data_quality.level
    );
    assert!(analysis.data_quality.validation_errors.is_empty());
    assert!(
        !analysis
            .data_quality
            .block_io_correlation_basis
            .trim()
            .is_empty(),
        "real_block_io_overlap must report block_io_correlation_basis"
    );
    assert_eq!(
        analysis.data_quality.block_io_correlation_basis, "request-pointer",
        "real_block_io_overlap should use strong request-pointer block I/O correlation"
    );
    assert!(
        analysis.artifacts_summary.block_io_event_count > 0,
        "real_block_io_overlap must contain block I/O artifacts"
    );
    assert!(
        analysis.artifacts_summary.block_io_event_count >= 2,
        "real_block_io_overlap should include one correlated block I/O event and one unrelated event outside the spike window"
    );
    assert_eq!(
        analysis.artifacts_summary.irq_event_count, 0,
        "IRQ evidence should not dominate real_block_io_overlap"
    );
    assert_eq!(
        analysis.artifacts_summary.gpu_sample_count, 0,
        "GPU evidence should not dominate real_block_io_overlap"
    );

    let candidate = find_candidate(&analysis, StutterCause::BlockIoCandidate)
        .expect("real_block_io_overlap missing block I/O diagnosis candidate");
    let candidate_evidence = candidate
        .evidence
        .iter()
        .map(|item| item.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        candidate_evidence.contains("block I/O"),
        "real_block_io_overlap block I/O candidate evidence did not mention block I/O; evidence was:\n{}",
        candidate_evidence
    );
    assert!(
        !candidate_evidence.contains("4,194,304")
            && !candidate_evidence.contains("4194304")
            && !candidate_evidence.contains("43ms"),
        "real_block_io_overlap block I/O evidence should stay focused on the correlated spike-window event and not report unrelated early I/O noise; evidence was:\n{}",
        candidate_evidence
    );
}

#[test]
fn validation_corpus_real_truncated_low_quality() {
    let analysis = assert_fixture_from_metadata("real_truncated_low_quality");

    assert!(
        matches!(
            analysis.data_quality.level,
            DataQualityLevel::Medium | DataQualityLevel::Low
        ),
        "real_truncated_low_quality data quality should be Medium or Low, got {:?}",
        analysis.data_quality.level
    );

    let has_low_quality_signal = analysis.data_quality.spike_events_truncated
        || analysis.data_quality.event_stream_write_errors > 0
        || analysis.data_quality.drop_counters_nonzero;
    assert!(
        has_low_quality_signal,
        "real_truncated_low_quality must expose truncation, event stream write errors, or nonzero drop counters"
    );

    let quality_text = analysis
        .data_quality
        .reasons
        .iter()
        .chain(analysis.data_quality.validation_warnings.iter())
        .chain(analysis.data_quality.validation_errors.iter())
        .map(|message| message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();

    assert!(
        quality_text.contains("truncated")
            || quality_text.contains("drop")
            || quality_text.contains("write error"),
        "real_truncated_low_quality quality reasons/warnings/errors must mention truncated, drop, or write error; text was:\n{}",
        quality_text
    );

    assert!(
        primary_diagnosis(&analysis).is_none() || no_primary_non_unknown_diagnosis(&analysis),
        "real_truncated_low_quality must not assert a strong non-Unknown diagnosis: {:?}",
        analysis
            .cluster_analysis
            .clusters
            .iter()
            .filter_map(|cluster| cluster.diagnosis.as_ref())
            .map(|diagnosis| &diagnosis.cause)
            .collect::<Vec<_>>()
    );

    assert!(
        analysis.data_quality.spike_events_truncated,
        "real_truncated_low_quality should exercise spike_events_truncated"
    );
    assert!(
        analysis.data_quality.spike_events_dropped_count > 0,
        "real_truncated_low_quality should exercise spike_events_dropped_count"
    );
    assert!(
        analysis.data_quality.drop_counters_nonzero,
        "real_truncated_low_quality should exercise drop_counters_nonzero"
    );
}

#[test]
fn validation_corpus_real_foreground_window() {
    let analysis = assert_fixture_from_metadata("real_foreground_window");

    assert!(
        analysis.foreground_summary.enabled,
        "real_foreground_window should report foreground tracking as enabled"
    );
    assert!(
        analysis.foreground_summary.event_count > 0,
        "real_foreground_window should contain at least one foreground event"
    );
    assert!(
        analysis.foreground_summary.final_pid.is_some()
            || analysis.foreground_summary.final_app_id.is_some()
            || analysis.foreground_summary.final_class.is_some(),
        "real_foreground_window should preserve final foreground pid, app_id, or class"
    );
    assert!(
        analysis.foreground_summary.final_title.is_none()
            || analysis.foreground_summary.final_title.as_deref() == Some("redacted"),
        "real_foreground_window title must be null or redacted, got {:?}",
        analysis.foreground_summary.final_title
    );
    assert_eq!(
        analysis.foreground_summary.final_pid,
        Some(stutter_core::ids::Pid::new(5701))
    );
    assert_eq!(
        analysis.foreground_summary.final_app_id.as_deref(),
        Some("steam_app_sanitized")
    );
    assert_eq!(
        analysis.foreground_summary.final_class.as_deref(),
        Some("steam_app_sanitized")
    );
    assert!(
        analysis.artifacts_summary.foreground_event_count > 0,
        "real_foreground_window must contain foreground_events.json artifacts"
    );
    assert!(
        analysis.artifacts_summary.spike_count >= 3,
        "real_foreground_window must contain a scheduler cluster near the foreground event"
    );

    let annotated_cluster = analysis.cluster_analysis.clusters.iter().find(|cluster| {
        cluster.foreground_pid == Some(5701)
            || cluster.foreground_app_id.as_deref() == Some("steam_app_sanitized")
            || cluster.foreground_class.as_deref() == Some("steam_app_sanitized")
    });
    assert!(
        annotated_cluster.is_some(),
        "real_foreground_window expected a cluster annotated with foreground pid/app/class; clusters={:?}",
        analysis.cluster_analysis.clusters
    );

    let cluster = annotated_cluster.expect("checked above");
    assert_eq!(cluster.foreground_pid, Some(5701));
    assert_eq!(
        cluster.foreground_app_id.as_deref(),
        Some("steam_app_sanitized")
    );
    assert_eq!(
        cluster.foreground_class.as_deref(),
        Some("steam_app_sanitized")
    );
    assert!(
        cluster.foreground_confidence.is_some(),
        "real_foreground_window annotated cluster should carry foreground confidence"
    );
}

#[test]
fn validation_corpus_real_community_rules_classification() {
    let analysis = assert_fixture_from_metadata("real_community_rules_classification");

    let classified_task = analysis
        .session
        .tasks
        .iter()
        .find(|task| task.comm == "community-game")
        .expect("missing community-rule-classified game task");

    assert_eq!(
        classified_task.class,
        crate::process_tree::TaskClass::Game,
        "report fixture should contain final class Game for the community-rule-classified task"
    );
    assert_eq!(classified_task.process_comm.as_str(), "community-game");

    assert_primary_anchor_class_in(
        &analysis,
        StutterCause::GameThreadSchedulerDelay,
        &[
            crate::process_tree::TaskClass::Game,
            crate::process_tree::TaskClass::GameRenderThread,
            crate::process_tree::TaskClass::GameWorkerThread,
            crate::process_tree::TaskClass::GameHelper,
            crate::process_tree::TaskClass::WineServer,
        ],
    );

    assert!(
        analysis.artifacts_summary.spike_count >= 3,
        "real_community_rules_classification should contain clustered game-relevant spikes"
    );
    assert!(
        analysis.artifacts_summary.frame_event_count > 0,
        "real_community_rules_classification should include frame context for downstream diagnosis"
    );
}
