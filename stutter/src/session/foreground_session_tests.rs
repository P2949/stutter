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
    crate::foreground::ForegroundWindowSnapshot {
        elapsed_ms: input.elapsed_ms,
        source: Some(crate::foreground::ForegroundSource::Sway),
        status: input.status,
        pid: input.pid,
        app_id: input.app_id.map(str::to_owned),
        class: input.class.map(str::to_owned),
        title: None,
        window_id: input.window_id.map(str::to_owned),
        workspace: input.workspace.map(str::to_owned),
        confidence: input.confidence,
        stale_ms: None,
        reason: "test foreground snapshot".to_owned(),
    }
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
    let old = crate::foreground::ForegroundWindowSnapshot {
        elapsed_ms: 100,
        source: Some(crate::foreground::ForegroundSource::X11),
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("Navigator".to_owned()),
        class: Some("Firefox".to_owned()),
        title: Some("old private title".to_owned()),
        window_id: Some("0x1200007".to_owned()),
        workspace: None,
        confidence: 0.90,
        stale_ms: None,
        reason: "old reason".to_owned(),
    };
    let new = crate::foreground::ForegroundWindowSnapshot {
        elapsed_ms: 250,
        source: Some(crate::foreground::ForegroundSource::X11),
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("Navigator".to_owned()),
        class: Some("Firefox".to_owned()),
        title: Some("new private title".to_owned()),
        window_id: Some("0x1200007".to_owned()),
        workspace: None,
        confidence: 0.50,
        stale_ms: None,
        reason: "new reason".to_owned(),
    };

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
    new.confidence = 0.50;
    new.reason = "using stale foreground snapshot from 500ms ago".to_owned();

    assert!(foreground_identity_changed(Some(&old), &new));

    old.stale_ms = Some(500);
    let mut still_stale = old.clone();
    still_stale.elapsed_ms = 1_000;
    still_stale.stale_ms = Some(900);

    assert!(!foreground_identity_changed(Some(&old), &still_stale));
}
