//! Foreground resolver tests extracted from `foreground`.
//!
//! Owns stale-snapshot, title-policy, confidence-retention, and resolver-default tests plus resolver-only fixtures.
//! Does not own provider parser behavior, event redaction model tests, or production foreground behavior.

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::super::{super::*, SequenceProvider};

    #[derive(Debug)]
    struct ScriptedForegroundProvider {
        source: ForegroundSource,
        samples: VecDeque<ForegroundWindowSnapshot>,
    }

    impl ScriptedForegroundProvider {
        fn new(source: ForegroundSource, samples: Vec<ForegroundWindowSnapshot>) -> Self {
            Self {
                source,
                samples: VecDeque::from(samples),
            }
        }
    }

    impl ForegroundProvider for ScriptedForegroundProvider {
        fn source(&self) -> ForegroundSource {
            self.source
        }

        fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
            self.samples.pop_front().unwrap_or_else(|| {
                ForegroundWindowSnapshot::unavailable(
                    elapsed_ms,
                    self.source,
                    "scripted provider has no more samples",
                )
            })
        }
    }

    #[test]
    fn foreground_resolver_returns_stale_snapshot_with_lower_confidence() {
        let good = ForegroundWindowSnapshot::available(ForegroundAvailableInput {
            elapsed_ms: 1_000,
            source: ForegroundSource::Sway,
            pid: Some(4242),
            app_id: Some("steam_app_379430".to_owned()),
            class: Some("steam_app_379430".to_owned()),
            title: Some("private title".to_owned()),
            include_title: true,
            window_id: Some("7".to_owned()),
            workspace: Some("gaming".to_owned()),
            confidence: 0.95,
            reason: "focused Sway node from swaymsg get_tree".to_owned(),
        });
        let mut error =
            ForegroundWindowSnapshot::unavailable(1_500, ForegroundSource::Sway, "swaymsg failed");
        error.status = ForegroundProviderStatus::Error;
        let provider = ScriptedForegroundProvider::new(ForegroundSource::Sway, vec![good, error]);
        let mut resolver = ForegroundResolver::new(Box::new(provider))
            .with_include_title(false)
            .with_max_stale_ms(2_500);

        let first = resolver.sample(1_000);
        let stale = resolver.sample(1_500);

        assert_eq!(first.status, ForegroundProviderStatus::Available);
        assert_eq!(stale.status, ForegroundProviderStatus::Available);
        assert_eq!(
            stale.decision.target.as_ref().and_then(|t| t.pid),
            Some(4242)
        );
        assert_eq!(
            stale.decision.target.as_ref().and_then(|t| t.title.clone()),
            None
        );
        assert_eq!(stale.stale_ms, Some(500));
        assert!(stale.decision.confidence < first.decision.confidence);
        assert!(stale.decision.reasons.iter().any(|reason| {
            reason
                .reason
                .contains("using stale foreground snapshot from 500ms ago")
        }));
    }

    #[test]
    fn foreground_resolver_drops_snapshot_after_max_stale() {
        let good = ForegroundWindowSnapshot::available(ForegroundAvailableInput {
            elapsed_ms: 1_000,
            source: ForegroundSource::X11,
            pid: Some(12345),
            app_id: Some("steam_app_379430".to_owned()),
            class: Some("steam_app_379430".to_owned()),
            title: Some("private title".to_owned()),
            include_title: true,
            window_id: Some("0x4600007".to_owned()),
            workspace: None,
            confidence: 0.90,
            reason: "active X11 window from xprop".to_owned(),
        });
        let mut error =
            ForegroundWindowSnapshot::unavailable(4_000, ForegroundSource::X11, "xprop failed");
        error.status = ForegroundProviderStatus::Error;
        let provider = ScriptedForegroundProvider::new(ForegroundSource::X11, vec![good, error]);
        let mut resolver = ForegroundResolver::new(Box::new(provider))
            .with_include_title(false)
            .with_max_stale_ms(1_000);

        let first = resolver.sample(1_000);
        let dropped = resolver.sample(4_000);

        assert_eq!(first.status, ForegroundProviderStatus::Available);
        assert_eq!(dropped.status, ForegroundProviderStatus::Error);
        assert_eq!(
            dropped
                .decision
                .target
                .as_ref()
                .and_then(|target| target.pid),
            None
        );
        assert_eq!(dropped.stale_ms, None);
        assert_eq!(dropped.decision.primary_reason(), Some("xprop failed"));
    }

    #[test]
    fn foreground_resolver_redacts_title_by_default() {
        let provider = SequenceProvider::new(
            ForegroundSource::Sway,
            vec![ForegroundWindowSnapshot::available(
                ForegroundAvailableInput {
                    elapsed_ms: 0,
                    source: ForegroundSource::Sway,
                    pid: Some(1234),
                    app_id: Some("firefox".to_owned()),
                    class: Some("Firefox".to_owned()),
                    title: Some("private browser tab".to_owned()),
                    include_title: true,
                    window_id: Some("42".to_owned()),
                    workspace: Some("web".to_owned()),
                    confidence: 0.95,
                    reason: "active sway node".to_owned(),
                },
            )],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider));

        let snapshot = resolver.sample(1_000);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.title.clone()),
            None
        );
        assert_eq!(
            snapshot.decision.target.as_ref().and_then(|t| t.pid),
            Some(1234)
        );
        assert_eq!(snapshot.decision.confidence, 0.95);
        assert_eq!(snapshot.stale_ms, None);
    }

    #[test]
    fn foreground_resolver_keeps_title_when_enabled() {
        let provider = SequenceProvider::new(
            ForegroundSource::Hyprland,
            vec![ForegroundWindowSnapshot::available(
                ForegroundAvailableInput {
                    elapsed_ms: 0,
                    source: ForegroundSource::Hyprland,
                    pid: Some(1234),
                    app_id: Some("foot".to_owned()),
                    class: Some("foot".to_owned()),
                    title: Some("terminal private title".to_owned()),
                    include_title: true,
                    window_id: Some("0xabc".to_owned()),
                    workspace: Some("dev".to_owned()),
                    confidence: 0.95,
                    reason: "active hyprland client".to_owned(),
                },
            )],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider)).with_include_title(true);

        let snapshot = resolver.sample(1_000);

        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.title.clone())
                .as_deref(),
            Some("terminal private title")
        );
    }

    #[test]
    fn foreground_resolver_returns_stale_last_good_snapshot_inside_window() {
        let provider = SequenceProvider::new(
            ForegroundSource::Sway,
            vec![
                ForegroundWindowSnapshot::available(ForegroundAvailableInput {
                    elapsed_ms: 0,
                    source: ForegroundSource::Sway,
                    pid: Some(2222),
                    app_id: Some("steam".to_owned()),
                    class: Some("Steam".to_owned()),
                    title: Some("private title".to_owned()),
                    include_title: true,
                    window_id: Some("17".to_owned()),
                    workspace: Some("games".to_owned()),
                    confidence: 0.95,
                    reason: "active sway node".to_owned(),
                }),
                ForegroundWindowSnapshot::unavailable(
                    0,
                    ForegroundSource::Sway,
                    "sway IPC timed out",
                ),
            ],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider));

        let first = resolver.sample(1_000);
        let stale = resolver.sample(2_000);

        assert_eq!(first.status, ForegroundProviderStatus::Available);
        assert_eq!(stale.status, ForegroundProviderStatus::Available);
        assert_eq!(
            stale.decision.target.as_ref().and_then(|t| t.pid),
            Some(2222)
        );
        assert_eq!(
            stale
                .decision
                .target
                .as_ref()
                .and_then(|t| t.app_id.clone())
                .as_deref(),
            Some("steam")
        );
        assert_eq!(
            stale.decision.target.as_ref().and_then(|t| t.title.clone()),
            None
        );
        assert_eq!(stale.stale_ms, Some(1_000));
        assert!(stale.decision.confidence < first.decision.confidence);
        assert!(stale.decision.reasons.iter().any(|reason| {
            reason
                .reason
                .contains("using stale foreground snapshot from 1000ms ago")
        }));
        assert!(
            stale
                .decision
                .reasons
                .iter()
                .any(|reason| reason.reason.contains("sway IPC timed out"))
        );
    }

    #[test]
    fn foreground_resolver_does_not_return_stale_snapshot_after_window() {
        let provider = SequenceProvider::new(
            ForegroundSource::Sway,
            vec![
                ForegroundWindowSnapshot::available(ForegroundAvailableInput {
                    elapsed_ms: 0,
                    source: ForegroundSource::Sway,
                    pid: Some(2222),
                    app_id: Some("steam".to_owned()),
                    class: Some("Steam".to_owned()),
                    title: None,
                    include_title: false,
                    window_id: Some("17".to_owned()),
                    workspace: Some("games".to_owned()),
                    confidence: 0.95,
                    reason: "active sway node".to_owned(),
                }),
                ForegroundWindowSnapshot::unavailable(
                    0,
                    ForegroundSource::Sway,
                    "sway IPC timed out",
                ),
            ],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider)).with_max_stale_ms(500);

        let first = resolver.sample(1_000);
        let second = resolver.sample(2_000);

        assert_eq!(first.status, ForegroundProviderStatus::Available);
        assert_eq!(second.status, ForegroundProviderStatus::Unavailable);
        assert_eq!(second.decision.primary_reason(), Some("sway IPC timed out"));
        assert_eq!(second.stale_ms, None);
    }

    #[test]
    fn foreground_resolver_retains_only_available_high_confidence_snapshots() {
        let provider = SequenceProvider::new(
            ForegroundSource::X11,
            vec![
                ForegroundWindowSnapshot::available(ForegroundAvailableInput {
                    elapsed_ms: 0,
                    source: ForegroundSource::X11,
                    pid: Some(3333),
                    app_id: None,
                    class: Some("Firefox".to_owned()),
                    title: None,
                    include_title: false,
                    window_id: Some("0x1200007".to_owned()),
                    workspace: Some("1".to_owned()),
                    confidence: 0.50,
                    reason: "low-confidence x11 focus".to_owned(),
                }),
                ForegroundWindowSnapshot::unavailable(
                    0,
                    ForegroundSource::X11,
                    "xprop unavailable",
                ),
            ],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider));

        let low_confidence = resolver.sample(1_000);
        let unavailable = resolver.sample(1_500);

        assert_eq!(low_confidence.status, ForegroundProviderStatus::Available);
        assert_eq!(low_confidence.decision.confidence, 0.50);
        assert!(resolver.last_snapshot().is_none());
        assert_eq!(unavailable.status, ForegroundProviderStatus::Unavailable);
        assert_eq!(
            unavailable.decision.primary_reason(),
            Some("xprop unavailable")
        );
    }

    #[test]
    fn foreground_resolver_defaults_match_phase_two_policy() {
        let provider = SequenceProvider::new(ForegroundSource::Unsupported, Vec::new());
        let resolver = ForegroundResolver::new(Box::new(provider));

        assert!(!resolver.include_title());
        assert_eq!(resolver.max_stale_ms(), 2_500);
        assert_eq!(resolver.provider_source(), ForegroundSource::Unsupported);
    }
}
