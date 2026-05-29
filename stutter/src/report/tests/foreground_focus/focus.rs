use super::*;

#[test]
fn focus_report_summary_prefers_latest_changed_focus_event() {
    let session = SessionFile {
        core: SessionMetadataCore {
            focus_mode: Some("auto-focus".to_owned()),
            final_focus_kind: Some("Browser".to_owned()),
            focus_switch_count: 2,
            ..Default::default()
        },
        config: RecordedConfig {
            auto_focus: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let events = vec![
        FocusEvent {
            elapsed_ms: 100,
            action: "changed".to_owned(),
            kind: Some("Browser".to_owned()),
            confidence: 0.62,
            situation: Some(SituationKind::BrowserFocused),
            root_pids: vec![111.into()],
            member_pids: vec![111.into(), 112.into()],
            reasons: vec!["browser parent with active renderer".to_owned()],
            ..Default::default()
        },
        FocusEvent {
            elapsed_ms: 200,
            action: "changed".to_owned(),
            kind: Some("Compile".to_owned()),
            confidence: 0.87,
            score: 0.91,
            situation: Some(SituationKind::CompileLoad),
            root_pids: vec![1234.into()],
            member_pids: vec![1234.into(), 1235.into()],
            reasons: vec![
                "cargo root with 14 active compiler descendants".to_owned(),
                "linker/write IO evidence observed".to_owned(),
            ],
            ..Default::default()
        },
    ];

    let summary = focus_report_summary(&session, &events);

    assert_eq!(summary.mode.as_deref(), Some("auto-focus"));
    assert_eq!(summary.final_focus.as_deref(), Some("Compile"));
    assert_eq!(summary.situation.as_deref(), Some("CompileLoad"));
    assert_eq!(summary.confidence, Some(0.87));
    assert_eq!(summary.score, Some(0.91));
    assert_eq!(summary.roots, vec![1234]);
    assert_eq!(summary.member_pids, vec![1234, 1235]);
    assert_eq!(summary.focus_switches, 2);
    assert_eq!(summary.reasons.len(), 2);
}

#[test]
fn render_focus_summary_text_includes_visible_reasons() {
    let summary = FocusReportSummary {
        mode: Some("auto-focus".to_owned()),
        final_focus: Some("Compile".to_owned()),
        display_name: Some("cargo build".to_owned()),
        situation: Some("CompileLoad".to_owned()),
        confidence: Some(0.87),
        score: Some(0.91),
        roots: vec![stutter_core::ids::Pid::new(1234)],
        member_pids: vec![
            stutter_core::ids::Pid::new(1234),
            stutter_core::ids::Pid::new(1235),
        ],
        focus_switches: 2,
        reasons: vec![
            "cargo root with 14 active compiler descendants".to_owned(),
            "CPU delta 780% over 1s".to_owned(),
        ],
    };

    let text = render_focus_summary_text(&summary);

    assert!(text.contains("Auto focus:"));
    assert!(text.contains("  mode: auto-focus"));
    assert!(text.contains("  final focus: Compile"));
    assert!(text.contains("  situation: CompileLoad"));
    assert!(text.contains("  confidence: 0.87"));
    assert!(text.contains("  roots: [1234]"));
    assert!(text.contains("  focus switches: 2"));
    assert!(text.contains("    - cargo root with 14 active compiler descendants"));
    assert!(text.contains("    - CPU delta 780% over 1s"));
}
