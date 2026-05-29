use super::*;

#[test]
fn report_includes_foreground_summary_when_events_present() {
    let mut session = minimal_session_for_report_test();
    session.config.foreground_window = true;
    session.config.foreground_source = "sway".to_owned();
    session.core.foreground_event_count = 1;

    let summary = foreground_report_summary(
        &session,
        &[foreground_event(
            1_000,
            Some(4242),
            Some("steam_app_379430"),
            Some("steam_app_379430"),
            None,
            Some("gaming"),
            0.95,
        )],
    );

    assert!(summary.enabled);
    assert_eq!(summary.source.as_deref(), Some("sway"));
    assert_eq!(summary.final_pid, Some(stutter_core::ids::Pid::new(4242)));
    assert_eq!(summary.final_app_id.as_deref(), Some("steam_app_379430"));
    assert_eq!(summary.final_class.as_deref(), Some("steam_app_379430"));
    assert_eq!(summary.final_window_id.as_deref(), Some("7"));
    assert_eq!(summary.final_workspace.as_deref(), Some("gaming"));
    assert_eq!(summary.event_count, 1);
    assert_eq!(summary.confidence, Some(0.95));
}

#[test]
fn report_redacts_missing_title_cleanly() {
    let summary = ForegroundReportSummary {
        enabled: true,
        source: Some("sway".to_owned()),
        final_pid: Some(stutter_core::ids::Pid::new(4242)),
        final_app_id: Some("steam_app_379430".to_owned()),
        final_class: Some("steam_app_379430".to_owned()),
        final_title: None,
        final_window_id: Some("7".to_owned()),
        final_workspace: Some("gaming".to_owned()),
        event_count: 1,
        confidence: Some(0.95),
        provider_status: Some("available".to_owned()),
        stale_ms: None,
        reasons: Vec::new(),
    };

    let text = render_foreground_summary_text(&summary);

    assert!(text.contains("Foreground window:"));
    assert!(text.contains("title: redacted (pass --foreground-include-title to record it)"));
    assert!(!text.contains("Private"));
}

#[test]
fn spike_cluster_gets_nearest_foreground_context() {
    let mut clusters = vec![cluster_at(1_500)];
    let events = vec![
        foreground_event(
            1_000,
            Some(1111),
            Some("steamwebhelper"),
            Some("steamwebhelper"),
            None,
            None,
            0.60,
        ),
        foreground_event(
            1_400,
            Some(4242),
            Some("steam_app_379430"),
            Some("steam_app_379430"),
            None,
            Some("gaming"),
            0.95,
        ),
        foreground_event(
            1_600,
            Some(9999),
            Some("future"),
            Some("future"),
            None,
            None,
            0.95,
        ),
    ];

    annotate_clusters_with_foreground(&mut clusters, &events, 1_000);

    assert_eq!(clusters[0].foreground_pid, Some(4242));
    assert_eq!(
        clusters[0].foreground_app_id.as_deref(),
        Some("steam_app_379430")
    );
    assert_eq!(
        clusters[0].foreground_class.as_deref(),
        Some("steam_app_379430")
    );
    assert_eq!(clusters[0].foreground_confidence, Some(0.95));
}

#[test]
fn foreground_report_summary_uses_final_event_and_redacted_title() {
    let mut session = minimal_session_for_report_test();
    session.config.foreground_window = true;
    session.config.foreground_source = "sway".to_owned();
    session.core.foreground_event_count = 2;
    session.core.foreground_source = Some("sway".to_owned());
    session.core.final_foreground_pid = Some(12345);
    session.core.final_foreground_app_id = Some("steam_app_379430".to_owned());
    session.core.final_foreground_class = Some("steam_app_379430".to_owned());

    let events = vec![
        foreground_event(
            100,
            Some(1000),
            Some("steam"),
            Some("Steam"),
            None,
            Some("gaming"),
            0.90,
        ),
        foreground_event(
            200,
            Some(12345),
            Some("steam_app_379430"),
            Some("steam_app_379430"),
            None,
            Some("gaming"),
            0.95,
        ),
    ];

    let summary = foreground_report_summary(&session, &events);

    assert!(summary.enabled);
    assert_eq!(summary.source.as_deref(), Some("sway"));
    assert_eq!(summary.final_pid, Some(stutter_core::ids::Pid::new(12345)));
    assert_eq!(summary.final_app_id.as_deref(), Some("steam_app_379430"));
    assert_eq!(summary.final_class.as_deref(), Some("steam_app_379430"));
    assert_eq!(summary.final_title, None);
    assert_eq!(summary.final_window_id.as_deref(), Some("7"));
    assert_eq!(summary.final_workspace.as_deref(), Some("gaming"));
    assert_eq!(summary.event_count, 2);
    assert_eq!(summary.confidence, Some(0.95));
    assert_eq!(summary.provider_status.as_deref(), Some("available"));
}

#[test]
fn render_foreground_summary_text_mentions_redacted_title() {
    let summary = ForegroundReportSummary {
        enabled: true,
        source: Some("sway".to_owned()),
        final_pid: Some(stutter_core::ids::Pid::new(12345)),
        final_app_id: Some("steam_app_379430".to_owned()),
        final_class: Some("steam_app_379430".to_owned()),
        final_title: None,
        final_window_id: Some("7".to_owned()),
        final_workspace: Some("gaming".to_owned()),
        event_count: 7,
        confidence: Some(0.95),
        provider_status: Some("available".to_owned()),
        stale_ms: None,
        reasons: vec!["focused Sway node from swaymsg get_tree".to_owned()],
    };

    let text = render_foreground_summary_text(&summary);

    assert!(text.contains("Foreground window:"));
    assert!(text.contains("  source: sway"));
    assert!(text.contains("  final pid: 12345"));
    assert!(text.contains("  app_id/class: steam_app_379430"));
    assert!(text.contains("  window_id: 7"));
    assert!(text.contains("  workspace: gaming"));
    assert!(text.contains("  confidence: 0.95"));
    assert!(text.contains("  stale: no"));
    assert!(text.contains("  events: 7"));
    assert!(text.contains("  title: redacted (pass --foreground-include-title to record it)"));
}

#[test]
fn foreground_for_cluster_uses_nearest_event_at_or_before_cluster_time() {
    let cluster = cluster_at(1_500);
    let events = vec![
        foreground_event(500, Some(1), Some("old"), Some("Old"), None, None, 0.50),
        foreground_event(
            1_200,
            Some(2),
            Some("game"),
            Some("Game"),
            None,
            Some("gaming"),
            0.95,
        ),
        foreground_event(
            1_600,
            Some(3),
            Some("future"),
            Some("Future"),
            None,
            None,
            0.95,
        ),
    ];

    let selected = foreground_for_cluster(&cluster, &events, 1_000).unwrap();

    assert_eq!(
        selected
            .decision
            .target
            .as_ref()
            .and_then(|target| target.pid),
        Some(2)
    );
    assert_eq!(
        selected
            .decision
            .target
            .as_ref()
            .and_then(|target| target.app_id.as_deref()),
        Some("game")
    );
}

#[test]
fn foreground_for_cluster_respects_max_stale_ms() {
    let cluster = cluster_at(2_000);
    let events = vec![foreground_event(
        500,
        Some(1),
        Some("old"),
        Some("Old"),
        None,
        None,
        0.50,
    )];

    assert!(foreground_for_cluster(&cluster, &events, 1_000).is_none());
}

#[test]
fn annotate_clusters_with_foreground_sets_cluster_fields() {
    let mut clusters = vec![cluster_at(1_500)];
    let events = vec![foreground_event(
        1_200,
        Some(12345),
        Some("steam_app_379430"),
        Some("steam_app_379430"),
        None,
        Some("gaming"),
        0.95,
    )];

    annotate_clusters_with_foreground(&mut clusters, &events, 1_000);

    assert_eq!(clusters[0].foreground_pid, Some(12345));
    assert_eq!(
        clusters[0].foreground_app_id.as_deref(),
        Some("steam_app_379430")
    );
    assert_eq!(
        clusters[0].foreground_class.as_deref(),
        Some("steam_app_379430")
    );
    assert_eq!(clusters[0].foreground_confidence, Some(0.95));
}

#[test]
fn report_analysis_json_contains_foreground_summary() {
    let mut session = minimal_session_for_report_test();
    session.config.foreground_window = true;
    session.config.foreground_source = "sway".to_owned();
    session.core.foreground_event_count = 1;
    let summary = foreground_report_summary(
        &session,
        &[foreground_event(
            100,
            Some(12345),
            Some("steam_app_379430"),
            Some("steam_app_379430"),
            None,
            Some("gaming"),
            0.95,
        )],
    );

    let json = serde_json::to_string(&summary).unwrap();

    assert!(json.contains("\"enabled\":true"));
    assert!(json.contains("\"source\":\"sway\""));
    assert!(json.contains("\"final_pid\":12345"));
    assert!(json.contains("\"event_count\":1"));
}
