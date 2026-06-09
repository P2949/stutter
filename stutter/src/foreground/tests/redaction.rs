//! Foreground snapshot and event redaction tests extracted from `foreground`.
//!
//! Owns snapshot/event title redaction, event conversion, and enum serialization tests.
//! Does not own provider parser behavior, resolver policy, or production foreground behavior.

#[cfg(test)]
mod tests {
    use super::super::super::*;

    #[test]
    fn snapshot_default_redacts_title_and_is_unsupported() {
        let snapshot = ForegroundWindowSnapshot::default();

        assert_eq!(snapshot.elapsed_ms, 0);
        assert_eq!(snapshot.source, None);
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unsupported);
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.title.clone()),
            None
        );
        assert_eq!(snapshot.decision.confidence, 0.0);
        assert_eq!(snapshot.stale_ms, None);
        assert_eq!(
            snapshot
                .decision
                .reasons
                .first()
                .map(|r| r.reason.clone())
                .unwrap_or_default(),
            ""
        );
    }

    #[test]
    fn available_snapshot_redacts_title_by_default() {
        let snapshot = ForegroundWindowSnapshot::available(ForegroundAvailableInput {
            elapsed_ms: 250,
            source: ForegroundSource::Sway,
            pid: Some(1234),
            app_id: Some("steam".to_owned()),
            class: Some("Steam".to_owned()),
            title: Some("Private chat title".to_owned()),
            include_title: false,
            window_id: Some("42".to_owned()),
            workspace: Some("games".to_owned()),
            confidence: 0.95,
            reason: "active sway node".to_owned(),
        });

        assert_eq!(snapshot.elapsed_ms, 250);
        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(
            snapshot.decision.target.as_ref().and_then(|t| t.pid),
            Some(1234)
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.app_id.clone())
                .as_deref(),
            Some("steam")
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.class.clone())
                .as_deref(),
            Some("Steam")
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.title.clone()),
            None
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.window_id.clone())
                .as_deref(),
            Some("42")
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.workspace.clone())
                .as_deref(),
            Some("games")
        );
        assert_eq!(snapshot.decision.confidence, 0.95);
        assert_eq!(
            snapshot
                .decision
                .reasons
                .first()
                .map(|r| r.reason.clone())
                .unwrap_or_default(),
            "active sway node"
        );
    }

    #[test]
    fn available_snapshot_keeps_title_when_explicitly_allowed() {
        let snapshot = ForegroundWindowSnapshot::available(ForegroundAvailableInput {
            elapsed_ms: 250,
            source: ForegroundSource::Hyprland,
            pid: Some(1234),
            app_id: Some("firefox".to_owned()),
            class: Some("firefox".to_owned()),
            title: Some("Private browser tab".to_owned()),
            include_title: true,
            window_id: Some("0xabc".to_owned()),
            workspace: Some("web".to_owned()),
            confidence: 0.90,
            reason: "active hyprland client".to_owned(),
        });

        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.title.clone())
                .as_deref(),
            Some("Private browser tab")
        );
    }

    #[test]
    fn event_constructor_redacts_title_by_default() {
        let event = ForegroundEvent::new(ForegroundEventInput {
            elapsed_ms: 500,
            source: ForegroundSource::X11,
            status: ForegroundProviderStatus::Available,
            pid: Some(5678),
            app_id: None,
            class: Some("Firefox".to_owned()),
            title: Some("Sensitive tab title".to_owned()),
            include_title: false,
            window_id: Some("0x1200007".to_owned()),
            workspace: Some("1".to_owned()),
            confidence: 0.80,
            stale_ms: None,
            reason: "active x11 window".to_owned(),
        });

        assert_eq!(
            event.decision.target.as_ref().and_then(|t| t.title.clone()),
            None
        );
        assert_eq!(event.source, ForegroundSource::X11);
        assert_eq!(event.status, ForegroundProviderStatus::Available);
    }

    #[test]
    fn event_from_snapshot_applies_title_policy_again() {
        let snapshot = ForegroundWindowSnapshot::available(ForegroundAvailableInput {
            elapsed_ms: 1_000,
            source: ForegroundSource::Sway,
            pid: Some(9000),
            app_id: Some("foot".to_owned()),
            class: None,
            title: Some("terminal: private path".to_owned()),
            include_title: true,
            window_id: Some("17".to_owned()),
            workspace: Some("dev".to_owned()),
            confidence: 1.0,
            reason: "test snapshot with title already present".to_owned(),
        });

        let redacted = ForegroundEvent::from_snapshot(&snapshot, false).unwrap();
        let included = ForegroundEvent::from_snapshot(&snapshot, true).unwrap();

        assert_eq!(
            redacted
                .decision
                .target
                .as_ref()
                .and_then(|t| t.title.clone()),
            None
        );
        assert_eq!(
            included
                .decision
                .target
                .as_ref()
                .and_then(|t| t.title.clone())
                .as_deref(),
            Some("terminal: private path")
        );
    }

    #[test]
    fn event_from_snapshot_requires_source() {
        let snapshot = ForegroundWindowSnapshot {
            source: None,
            status: ForegroundProviderStatus::Unavailable,
            decision: crate::foreground::model::ForegroundDecision::new(
                None,
                0.0,
                "no provider selected",
            ),
            ..ForegroundWindowSnapshot::default()
        };

        assert!(ForegroundEvent::from_snapshot(&snapshot, false).is_none());
    }

    #[test]
    fn serde_uses_snake_case_for_enums() {
        let event = ForegroundEvent::new(ForegroundEventInput {
            elapsed_ms: 100,
            source: ForegroundSource::Hyprland,
            status: ForegroundProviderStatus::Unavailable,
            pid: None,
            app_id: None,
            class: None,
            title: None,
            include_title: false,
            window_id: None,
            workspace: None,
            confidence: 0.0,
            stale_ms: None,
            reason: "hyprctl unavailable".to_owned(),
        });

        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains(r#""source":"hyprland""#));
        assert!(json.contains(r#""status":"unavailable""#));
    }
}
