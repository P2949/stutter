//! Foreground and focus report summaries.
//!
//! Owns foreground/focus summary construction and annotation of spike clusters with foreground
//! context. Does not own artifact loading, timing summaries, clustering, diagnosis, or report
//! orchestration.

use super::*;

pub(crate) fn foreground_report_summary(
    session: &SessionFile,
    foreground_events: &[ForegroundEvent],
) -> ForegroundReportSummary {
    let final_event = foreground_events.last();

    let enabled = session.config.foreground_window || session.core.foreground_event_count > 0;
    let source = final_event
        .map(|event| format!("{:?}", event.source).to_ascii_lowercase())
        .or_else(|| session.core.foreground_source.clone())
        .or_else(|| {
            (!session.config.foreground_source.is_empty())
                .then(|| session.config.foreground_source.clone())
        });

    let final_pid = final_event
        .and_then(|event| event.pid)
        .or(session.core.final_foreground_pid);
    let final_app_id = final_event
        .and_then(|event| event.app_id.clone())
        .or_else(|| session.core.final_foreground_app_id.clone());
    let final_class = final_event
        .and_then(|event| event.class.clone())
        .or_else(|| session.core.final_foreground_class.clone());
    let final_title = final_event.and_then(|event| event.title.clone());
    let final_window_id = final_event
        .and_then(|event| event.window_id.clone())
        .or_else(|| session.core.final_foreground_window_id.clone());
    let final_workspace = final_event
        .and_then(|event| event.workspace.clone())
        .or_else(|| session.core.final_foreground_workspace.clone());
    let confidence = final_event
        .map(|event| event.confidence)
        .or(session.core.final_foreground_confidence);
    let provider_status = final_event
        .map(|event| format!("{:?}", event.status).to_ascii_lowercase())
        .or_else(|| session.core.final_foreground_status.clone());
    let stale_ms = final_event
        .and_then(|event| event.stale_ms)
        .or(session.core.final_foreground_stale_ms);
    let reasons = final_event
        .map(|event| vec![event.reason.clone()])
        .or_else(|| {
            session
                .core
                .final_foreground_reason
                .clone()
                .map(|reason| vec![reason])
        })
        .unwrap_or_default();

    ForegroundReportSummary {
        enabled,
        source,
        final_pid,
        final_app_id,
        final_class,
        final_title,
        final_window_id,
        final_workspace,
        event_count: session
            .core
            .foreground_event_count
            .max(foreground_events.len() as u64),
        confidence,
        provider_status,
        stale_ms,
        reasons,
    }
}

pub(crate) fn focus_report_summary(
    session: &SessionFile,
    focus_events: &[FocusEvent],
) -> FocusReportSummary {
    let final_event = focus_events
        .iter()
        .rev()
        .find(|event| event.action == "changed" || event.kind.is_some());

    let mode = session
        .core
        .focus_mode
        .clone()
        .or_else(|| session.config.auto_focus.then(|| "auto-focus".to_owned()));

    let final_focus = final_event
        .and_then(|event| event.kind.clone())
        .or_else(|| session.core.final_focus_kind.clone());

    let situation =
        final_event.and_then(|event| event.situation.map(|situation| format!("{situation:?}")));

    let confidence = final_event.map(|event| event.confidence);
    let score = final_event.map(|event| event.score);
    let roots = final_event
        .map(|event| event.root_pids.iter().map(|pid| pid.as_u32()).collect())
        .unwrap_or_default();
    let member_pids = final_event
        .map(|event| event.member_pids.iter().map(|pid| pid.as_u32()).collect())
        .unwrap_or_default();
    let reasons = final_event
        .map(|event| event.reasons.clone())
        .unwrap_or_default();

    let display_name = final_focus.clone();

    FocusReportSummary {
        mode,
        final_focus,
        display_name,
        situation,
        confidence,
        score,
        roots,
        member_pids,
        focus_switches: session.core.focus_switch_count,
        reasons,
    }
}

pub(crate) fn annotate_clusters_with_foreground(
    clusters: &mut [SpikeCluster],
    foreground_events: &[ForegroundEvent],
    max_stale_ms: u64,
) {
    for cluster in clusters {
        if let Some(event) = foreground_for_cluster(cluster, foreground_events, max_stale_ms) {
            cluster.foreground_pid = event.pid;
            cluster.foreground_app_id = event.app_id.clone();
            cluster.foreground_class = event.class.clone();
            cluster.foreground_confidence = Some(event.confidence);
        }
    }
}

pub(crate) fn foreground_for_cluster<'a>(
    cluster: &SpikeCluster,
    foreground_events: &'a [ForegroundEvent],
    max_stale_ms: u64,
) -> Option<&'a ForegroundEvent> {
    foreground_for_elapsed_ms(
        cluster_elapsed_ms(cluster)?,
        foreground_events,
        max_stale_ms,
    )
}

pub(crate) fn foreground_for_elapsed_ms(
    elapsed_ms: u64,
    foreground_events: &[ForegroundEvent],
    max_stale_ms: u64,
) -> Option<&ForegroundEvent> {
    foreground_events
        .iter()
        .filter(|event| event.elapsed_ms <= elapsed_ms)
        .filter(|event| elapsed_ms.saturating_sub(event.elapsed_ms) <= max_stale_ms)
        .max_by_key(|event| event.elapsed_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        foreground::{ForegroundProviderStatus, ForegroundSource},
        recorder::{ForegroundEvent, SessionFile},
    };

    #[test]
    fn foreground_report_summary_falls_back_to_session_core_full_final_identity() {
        let mut session = SessionFile::default();
        session.config.foreground_window = true;
        session.core.foreground_event_count = 1;
        session.core.foreground_source = Some("sway".to_owned());
        session.core.final_foreground_pid = Some(159447);
        session.core.final_foreground_window_id = Some("163".to_owned());
        session.core.final_foreground_workspace = Some("5".to_owned());
        session.core.final_foreground_status = Some("available".to_owned());
        session.core.final_foreground_confidence = Some(0.95);
        session.core.final_foreground_stale_ms = Some(500);
        session.core.final_foreground_reason =
            Some("focused Sway node from swaymsg get_tree".to_owned());

        let summary = foreground_report_summary(&session, &[]);

        assert!(summary.enabled);
        assert_eq!(summary.source.as_deref(), Some("sway"));
        assert_eq!(summary.final_pid, Some(159447));
        assert_eq!(summary.final_window_id.as_deref(), Some("163"));
        assert_eq!(summary.final_workspace.as_deref(), Some("5"));
        assert_eq!(summary.provider_status.as_deref(), Some("available"));
        assert_eq!(summary.confidence, Some(0.95));
        assert_eq!(summary.stale_ms, Some(500));
        assert_eq!(summary.reasons.len(), 1);
    }

    #[test]
    fn foreground_report_summary_prefers_event_artifact_over_session_core() {
        let mut session = SessionFile::default();
        session.core.final_foreground_pid = Some(1111);
        session.core.final_foreground_window_id = Some("old".to_owned());

        let events = vec![ForegroundEvent {
            elapsed_ms: 1_000,
            source: ForegroundSource::Sway,
            status: ForegroundProviderStatus::Available,
            pid: Some(2222),
            app_id: Some("Alacritty".to_owned()),
            class: None,
            title: None,
            window_id: Some("new".to_owned()),
            workspace: Some("4".to_owned()),
            confidence: 0.95,
            stale_ms: None,
            reason: "new foreground event".to_owned(),
        }];

        let summary = foreground_report_summary(&session, &events);

        assert_eq!(summary.final_pid, Some(2222));
        assert_eq!(summary.final_window_id.as_deref(), Some("new"));
    }
}
