//! Tests for session foreground identity-change behavior.
//!
//! Owns foreground session regression tests and test-only snapshot builders. Does not own
//! production session runtime, tick handling, or foreground provider behavior.

use super::*;

struct ForegroundSnapshotTestInput<'a> {
    elapsed_ms: u64,
    status: crate::foreground::ForegroundProviderStatus,
    pid: Option<u32>,
    app_id: Option<&'a str>,
    class: Option<&'a str>,
    window_id: Option<&'a str>,
    workspace: Option<&'a str>,
    confidence: f32,
}

fn foreground_snapshot(
    input: ForegroundSnapshotTestInput<'_>,
) -> crate::foreground::ForegroundWindowSnapshot {
    let mut snapshot = crate::foreground::ForegroundWindowSnapshot::available(
        crate::foreground::ForegroundAvailableInput {
            elapsed_ms: input.elapsed_ms,
            source: crate::foreground::ForegroundSource::Sway,
            pid: input.pid,
            app_id: input.app_id.map(str::to_owned),
            class: input.class.map(str::to_owned),
            title: None,
            include_title: false,
            window_id: input.window_id.map(str::to_owned),
            workspace: input.workspace.map(str::to_owned),
            confidence: input.confidence,
            reason: "test foreground snapshot".to_owned(),
        },
    );
    snapshot.status = input.status;
    snapshot
}

#[test]
fn foreground_identity_records_first_sample() {
    let snapshot = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 100,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("steam"),
        class: Some("Steam"),
        window_id: Some("7"),
        workspace: Some("games"),
        confidence: 0.95,
    });

    assert!(foreground_identity_changed(None, &snapshot));
}

#[test]
fn foreground_identity_changes_on_provider_status_transition() {
    let old = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 100,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("steam"),
        class: Some("Steam"),
        window_id: Some("7"),
        workspace: Some("games"),
        confidence: 0.95,
    });
    let new = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 200,
        status: crate::foreground::ForegroundProviderStatus::Error,
        pid: Some(4242),
        app_id: Some("steam"),
        class: Some("Steam"),
        window_id: Some("7"),
        workspace: Some("games"),
        confidence: 0.0,
    });

    assert!(foreground_identity_changed(Some(&old), &new));
}

#[test]
fn foreground_identity_changes_on_window_identity_transition() {
    let old = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 100,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("steam"),
        class: Some("Steam"),
        window_id: Some("7"),
        workspace: Some("games"),
        confidence: 0.95,
    });
    let new = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 200,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(9000),
        app_id: Some("firefox"),
        class: Some("Firefox"),
        window_id: Some("8"),
        workspace: Some("web"),
        confidence: 0.95,
    });

    assert!(foreground_identity_changed(Some(&old), &new));
}

#[test]
fn foreground_identity_ignores_elapsed_title_reason_and_confidence_only_changes() {
    let mut old = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 100,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("Navigator"),
        class: Some("Firefox"),
        window_id: Some("0x1200007"),
        workspace: None,
        confidence: 0.90,
    });
    old.source = Some(crate::foreground::ForegroundSource::X11);
    if let Some(target) = old.decision.target.as_mut() {
        target.title = Some("old private title".to_owned());
    }
    old.decision.reasons = vec![crate::foreground::ForegroundReason::new("old reason")];

    let mut new = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 250,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("Navigator"),
        class: Some("Firefox"),
        window_id: Some("0x1200007"),
        workspace: None,
        confidence: 0.50,
    });
    new.source = Some(crate::foreground::ForegroundSource::X11);
    if let Some(target) = new.decision.target.as_mut() {
        target.title = Some("new private title".to_owned());
    }
    new.decision.reasons = vec![crate::foreground::ForegroundReason::new("new reason")];

    assert!(!foreground_identity_changed(Some(&old), &new));
}

#[test]
fn foreground_identity_changes_when_same_pid_moves_to_different_window_id() {
    let old = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 100,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(2960),
        app_id: Some("steam"),
        class: None,
        window_id: Some("25"),
        workspace: Some("4"),
        confidence: 0.95,
    });

    let new = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 200,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(2960),
        app_id: Some("Spotify"),
        class: None,
        window_id: Some("7"),
        workspace: Some("2"),
        confidence: 0.95,
    });

    assert!(foreground_identity_changed(Some(&old), &new));
}

#[test]
fn foreground_identity_changes_when_fresh_snapshot_becomes_stale_once() {
    let mut old = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 100,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("steam"),
        class: Some("Steam"),
        window_id: Some("42"),
        workspace: Some("games"),
        confidence: 0.95,
    });

    let mut new = old.clone();
    new.elapsed_ms = 600;
    new.stale_ms = Some(500);
    new.decision.confidence = 0.50;
    new.decision.reasons = vec![crate::foreground::ForegroundReason::new(
        "using stale foreground snapshot from 500ms ago",
    )];

    assert!(foreground_identity_changed(Some(&old), &new));

    old.stale_ms = Some(500);
    let mut still_stale = old.clone();
    still_stale.elapsed_ms = 1_000;
    still_stale.stale_ms = Some(900);

    assert!(!foreground_identity_changed(Some(&old), &still_stale));
}

#[test]
fn final_foreground_metadata_prefers_current_snapshot_when_stale_age_did_not_emit() {
    let mut last_recorded_snapshot = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 600,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("steam"),
        class: Some("Steam"),
        window_id: Some("42"),
        workspace: Some("games"),
        confidence: 0.50,
    });
    last_recorded_snapshot.stale_ms = Some(500);
    last_recorded_snapshot.decision.reasons = vec![crate::foreground::ForegroundReason::new(
        "using stale foreground snapshot from 500ms ago",
    )];

    let mut current_snapshot = last_recorded_snapshot.clone();
    current_snapshot.elapsed_ms = 2_100;
    current_snapshot.stale_ms = Some(2_000);
    current_snapshot.decision.confidence = 0.25;
    current_snapshot.decision.reasons = vec![crate::foreground::ForegroundReason::new(
        "using stale foreground snapshot from 2000ms ago",
    )];

    assert!(!foreground_identity_changed(
        Some(&last_recorded_snapshot),
        &current_snapshot
    ));

    let last_recorded_event = last_recorded_snapshot.to_event(false).unwrap();
    let final_event = crate::session::ticks::foreground::foreground_event_for_final_metadata(
        Some(&current_snapshot),
        Some(&last_recorded_event),
        false,
    )
    .unwrap();

    assert_eq!(final_event.elapsed_ms, 2_100);
    assert_eq!(
        final_event.decision.target.as_ref().and_then(|t| t.pid),
        Some(4242)
    );
    assert_eq!(
        final_event
            .decision
            .target
            .as_ref()
            .and_then(|t| t.window_id.clone())
            .as_deref(),
        Some("42")
    );
    assert_eq!(final_event.stale_ms, Some(2_000));
    assert_eq!(final_event.decision.confidence, 0.25);
    assert_eq!(
        final_event
            .decision
            .reasons
            .first()
            .map(|r| r.reason.clone())
            .unwrap_or_default()
            .as_str(),
        "using stale foreground snapshot from 2000ms ago"
    );
}

#[test]
fn final_foreground_metadata_falls_back_to_last_recorded_event_without_current_snapshot() {
    let snapshot = foreground_snapshot(ForegroundSnapshotTestInput {
        elapsed_ms: 600,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("steam"),
        class: Some("Steam"),
        window_id: Some("42"),
        workspace: Some("games"),
        confidence: 0.95,
    });
    let last_recorded_event = snapshot.to_event(false).unwrap();

    let final_event = crate::session::ticks::foreground::foreground_event_for_final_metadata(
        None,
        Some(&last_recorded_event),
        false,
    )
    .unwrap();

    assert_eq!(final_event.elapsed_ms, 600);
    assert_eq!(
        final_event.decision.target.as_ref().and_then(|t| t.pid),
        Some(4242)
    );
    assert_eq!(
        final_event
            .decision
            .target
            .as_ref()
            .and_then(|t| t.window_id.clone())
            .as_deref(),
        Some("42")
    );
}
