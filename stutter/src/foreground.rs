#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForegroundSource {
    Auto,
    Sway,
    Hyprland,
    X11,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ForegroundProviderStatus {
    Available,
    Unavailable,
    Error,
    #[default]
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ForegroundWindowSnapshot {
    pub elapsed_ms: u64,

    pub source: Option<ForegroundSource>,
    pub status: ForegroundProviderStatus,

    pub pid: Option<u32>,

    // Wayland app_id, Hyprland class, X11 WM_CLASS, etc.
    pub app_id: Option<String>,
    pub class: Option<String>,

    // Redacted unless --foreground-include-title is passed.
    pub title: Option<String>,

    pub window_id: Option<String>,
    pub workspace: Option<String>,

    pub confidence: f32,
    pub stale_ms: Option<u64>,
    pub reason: String,
}

impl ForegroundWindowSnapshot {
    pub fn unsupported(elapsed_ms: u64, reason: impl Into<String>) -> Self {
        Self {
            elapsed_ms,
            source: Some(ForegroundSource::Unsupported),
            status: ForegroundProviderStatus::Unsupported,
            reason: reason.into(),
            ..Self::default()
        }
    }

    pub fn unavailable(
        elapsed_ms: u64,
        source: ForegroundSource,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            elapsed_ms,
            source: Some(source),
            status: ForegroundProviderStatus::Unavailable,
            reason: reason.into(),
            ..Self::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn available(
        elapsed_ms: u64,
        source: ForegroundSource,
        pid: Option<u32>,
        app_id: Option<String>,
        class: Option<String>,
        title: Option<String>,
        include_title: bool,
        window_id: Option<String>,
        workspace: Option<String>,
        confidence: f32,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            elapsed_ms,
            source: Some(source),
            status: ForegroundProviderStatus::Available,
            pid,
            app_id,
            class,
            title: redact_title_unless_allowed(title, include_title),
            window_id,
            workspace,
            confidence,
            stale_ms: None,
            reason: reason.into(),
        }
    }

    pub fn with_title_policy(mut self, title: Option<String>, include_title: bool) -> Self {
        self.title = redact_title_unless_allowed(title, include_title);
        self
    }

    pub fn redact_title(mut self) -> Self {
        self.title = None;
        self
    }

    pub fn to_event(&self, include_title: bool) -> Option<ForegroundEvent> {
        let source = self.source?;

        Some(ForegroundEvent {
            elapsed_ms: self.elapsed_ms,
            source,
            status: self.status,
            pid: self.pid,
            app_id: self.app_id.clone(),
            class: self.class.clone(),
            title: redact_title_unless_allowed(self.title.clone(), include_title),
            window_id: self.window_id.clone(),
            workspace: self.workspace.clone(),
            confidence: self.confidence,
            reason: self.reason.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForegroundEvent {
    pub elapsed_ms: u64,
    pub source: ForegroundSource,
    pub status: ForegroundProviderStatus,
    pub pid: Option<u32>,
    pub app_id: Option<String>,
    pub class: Option<String>,
    pub title: Option<String>,
    pub window_id: Option<String>,
    pub workspace: Option<String>,
    pub confidence: f32,
    pub reason: String,
}

impl ForegroundEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        elapsed_ms: u64,
        source: ForegroundSource,
        status: ForegroundProviderStatus,
        pid: Option<u32>,
        app_id: Option<String>,
        class: Option<String>,
        title: Option<String>,
        include_title: bool,
        window_id: Option<String>,
        workspace: Option<String>,
        confidence: f32,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            elapsed_ms,
            source,
            status,
            pid,
            app_id,
            class,
            title: redact_title_unless_allowed(title, include_title),
            window_id,
            workspace,
            confidence,
            reason: reason.into(),
        }
    }

    pub fn from_snapshot(snapshot: &ForegroundWindowSnapshot, include_title: bool) -> Option<Self> {
        snapshot.to_event(include_title)
    }

    pub fn redact_title(mut self) -> Self {
        self.title = None;
        self
    }
}

pub fn redact_title_unless_allowed(title: Option<String>, include_title: bool) -> Option<String> {
    if include_title { title } else { None }
}

pub const DEFAULT_FOREGROUND_POLL_MS: u64 = 1_000;
pub const DEFAULT_FOREGROUND_MAX_STALE_MS: u64 = 2_500;
pub const DEFAULT_FOREGROUND_MIN_CONFIDENCE: f32 = 0.75;
pub const DEFAULT_FOREGROUND_INCLUDE_TITLE: bool = false;

pub trait ForegroundProvider {
    fn source(&self) -> ForegroundSource;
    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot;
}

pub struct ForegroundResolver {
    provider: Box<dyn ForegroundProvider + Send>,
    include_title: bool,
    last_snapshot: Option<ForegroundWindowSnapshot>,
    max_stale_ms: u64,
}

impl ForegroundResolver {
    pub fn new(provider: Box<dyn ForegroundProvider + Send>) -> Self {
        Self {
            provider,
            include_title: DEFAULT_FOREGROUND_INCLUDE_TITLE,
            last_snapshot: None,
            max_stale_ms: DEFAULT_FOREGROUND_MAX_STALE_MS,
        }
    }

    pub fn with_include_title(mut self, include_title: bool) -> Self {
        self.include_title = include_title;
        self
    }

    pub fn with_max_stale_ms(mut self, max_stale_ms: u64) -> Self {
        self.max_stale_ms = max_stale_ms;
        self
    }

    pub fn include_title(&self) -> bool {
        self.include_title
    }

    pub fn max_stale_ms(&self) -> u64 {
        self.max_stale_ms
    }

    pub fn last_snapshot(&self) -> Option<&ForegroundWindowSnapshot> {
        self.last_snapshot.as_ref()
    }

    pub fn provider_source(&self) -> ForegroundSource {
        self.provider.source()
    }

    pub fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
        let mut snapshot = self.provider.sample(elapsed_ms);
        snapshot.source = snapshot.source.or(Some(self.provider.source()));
        snapshot.title = redact_title_unless_allowed(snapshot.title, self.include_title);

        if is_good_foreground_snapshot(&snapshot) {
            snapshot.stale_ms = None;
            self.last_snapshot = Some(snapshot.clone());
            return snapshot;
        }

        if let Some(stale) = self.stale_snapshot(elapsed_ms, &snapshot.reason) {
            return stale;
        }

        snapshot
    }

    fn stale_snapshot(
        &self,
        elapsed_ms: u64,
        failed_reason: &str,
    ) -> Option<ForegroundWindowSnapshot> {
        let last = self.last_snapshot.as_ref()?;
        let stale_ms = elapsed_ms.checked_sub(last.elapsed_ms)?;

        if stale_ms > self.max_stale_ms {
            return None;
        }

        let mut snapshot = last.clone();
        snapshot.elapsed_ms = elapsed_ms;
        snapshot.title = redact_title_unless_allowed(snapshot.title, self.include_title);
        snapshot.confidence =
            reduce_stale_confidence(snapshot.confidence, stale_ms, self.max_stale_ms);
        snapshot.stale_ms = Some(stale_ms);
        snapshot.reason = if failed_reason.trim().is_empty() {
            format!("using stale foreground snapshot from {}ms ago", stale_ms)
        } else {
            format!(
                "using stale foreground snapshot from {}ms ago after provider sample failed: {}",
                stale_ms, failed_reason
            )
        };

        Some(snapshot)
    }
}

fn is_good_foreground_snapshot(snapshot: &ForegroundWindowSnapshot) -> bool {
    snapshot.status == ForegroundProviderStatus::Available
        && snapshot.source.is_some()
        && snapshot.confidence >= DEFAULT_FOREGROUND_MIN_CONFIDENCE
}

fn reduce_stale_confidence(confidence: f32, stale_ms: u64, max_stale_ms: u64) -> f32 {
    if max_stale_ms == 0 {
        return 0.0;
    }

    let stale_fraction = (stale_ms as f32 / max_stale_ms as f32).clamp(0.0, 1.0);
    let multiplier = 0.75 - (0.50 * stale_fraction);
    (confidence * multiplier).clamp(0.0, confidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SequenceProvider {
        source: ForegroundSource,
        snapshots: Vec<ForegroundWindowSnapshot>,
        index: usize,
    }

    impl SequenceProvider {
        fn new(source: ForegroundSource, snapshots: Vec<ForegroundWindowSnapshot>) -> Self {
            Self {
                source,
                snapshots,
                index: 0,
            }
        }
    }

    impl ForegroundProvider for SequenceProvider {
        fn source(&self) -> ForegroundSource {
            self.source
        }

        fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
            let mut snapshot = self.snapshots.get(self.index).cloned().unwrap_or_else(|| {
                ForegroundWindowSnapshot::unavailable(
                    elapsed_ms,
                    self.source,
                    "sequence provider exhausted",
                )
            });

            self.index = self.index.saturating_add(1);
            snapshot.elapsed_ms = elapsed_ms;
            snapshot
        }
    }

    #[test]
    fn foreground_resolver_redacts_title_by_default() {
        let provider = SequenceProvider::new(
            ForegroundSource::Sway,
            vec![ForegroundWindowSnapshot::available(
                0,
                ForegroundSource::Sway,
                Some(1234),
                Some("firefox".to_owned()),
                Some("Firefox".to_owned()),
                Some("private browser tab".to_owned()),
                true,
                Some("42".to_owned()),
                Some("web".to_owned()),
                0.95,
                "active sway node",
            )],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider));

        let snapshot = resolver.sample(1_000);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.title, None);
        assert_eq!(snapshot.pid, Some(1234));
        assert_eq!(snapshot.confidence, 0.95);
        assert_eq!(snapshot.stale_ms, None);
    }

    #[test]
    fn foreground_resolver_keeps_title_when_enabled() {
        let provider = SequenceProvider::new(
            ForegroundSource::Hyprland,
            vec![ForegroundWindowSnapshot::available(
                0,
                ForegroundSource::Hyprland,
                Some(1234),
                Some("foot".to_owned()),
                Some("foot".to_owned()),
                Some("terminal private title".to_owned()),
                true,
                Some("0xabc".to_owned()),
                Some("dev".to_owned()),
                0.95,
                "active hyprland client",
            )],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider)).with_include_title(true);

        let snapshot = resolver.sample(1_000);

        assert_eq!(snapshot.title.as_deref(), Some("terminal private title"));
    }

    #[test]
    fn foreground_resolver_returns_stale_last_good_snapshot_inside_window() {
        let provider = SequenceProvider::new(
            ForegroundSource::Sway,
            vec![
                ForegroundWindowSnapshot::available(
                    0,
                    ForegroundSource::Sway,
                    Some(2222),
                    Some("steam".to_owned()),
                    Some("Steam".to_owned()),
                    Some("private title".to_owned()),
                    true,
                    Some("17".to_owned()),
                    Some("games".to_owned()),
                    0.95,
                    "active sway node",
                ),
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
        assert_eq!(stale.pid, Some(2222));
        assert_eq!(stale.app_id.as_deref(), Some("steam"));
        assert_eq!(stale.title, None);
        assert_eq!(stale.stale_ms, Some(1_000));
        assert!(stale.confidence < first.confidence);
        assert!(
            stale
                .reason
                .contains("using stale foreground snapshot from 1000ms ago")
        );
        assert!(stale.reason.contains("sway IPC timed out"));
    }

    #[test]
    fn foreground_resolver_does_not_return_stale_snapshot_after_window() {
        let provider = SequenceProvider::new(
            ForegroundSource::Sway,
            vec![
                ForegroundWindowSnapshot::available(
                    0,
                    ForegroundSource::Sway,
                    Some(2222),
                    Some("steam".to_owned()),
                    Some("Steam".to_owned()),
                    None,
                    false,
                    Some("17".to_owned()),
                    Some("games".to_owned()),
                    0.95,
                    "active sway node",
                ),
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
        assert_eq!(second.reason, "sway IPC timed out");
        assert_eq!(second.stale_ms, None);
    }

    #[test]
    fn foreground_resolver_retains_only_available_high_confidence_snapshots() {
        let provider = SequenceProvider::new(
            ForegroundSource::X11,
            vec![
                ForegroundWindowSnapshot::available(
                    0,
                    ForegroundSource::X11,
                    Some(3333),
                    None,
                    Some("Firefox".to_owned()),
                    None,
                    false,
                    Some("0x1200007".to_owned()),
                    Some("1".to_owned()),
                    0.50,
                    "low-confidence x11 focus",
                ),
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
        assert_eq!(low_confidence.confidence, 0.50);
        assert!(resolver.last_snapshot().is_none());
        assert_eq!(unavailable.status, ForegroundProviderStatus::Unavailable);
        assert_eq!(unavailable.reason, "xprop unavailable");
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn foreground_resolver_defaults_match_phase_two_policy() {
        let provider = SequenceProvider::new(ForegroundSource::Unsupported, Vec::new());
        let resolver = ForegroundResolver::new(Box::new(provider));

        assert_eq!(DEFAULT_FOREGROUND_POLL_MS, 1_000);
        assert_eq!(DEFAULT_FOREGROUND_MAX_STALE_MS, 2_500);
        assert_eq!(DEFAULT_FOREGROUND_MIN_CONFIDENCE, 0.75);
        assert!(!DEFAULT_FOREGROUND_INCLUDE_TITLE);
        assert!(!resolver.include_title());
        assert_eq!(resolver.max_stale_ms(), 2_500);
        assert_eq!(resolver.provider_source(), ForegroundSource::Unsupported);
    }

    #[test]
    fn snapshot_default_redacts_title_and_is_unsupported() {
        let snapshot = ForegroundWindowSnapshot::default();

        assert_eq!(snapshot.elapsed_ms, 0);
        assert_eq!(snapshot.source, None);
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unsupported);
        assert_eq!(snapshot.title, None);
        assert_eq!(snapshot.confidence, 0.0);
        assert_eq!(snapshot.stale_ms, None);
        assert_eq!(snapshot.reason, "");
    }

    #[test]
    fn available_snapshot_redacts_title_by_default() {
        let snapshot = ForegroundWindowSnapshot::available(
            250,
            ForegroundSource::Sway,
            Some(1234),
            Some("steam".to_owned()),
            Some("Steam".to_owned()),
            Some("Private chat title".to_owned()),
            false,
            Some("42".to_owned()),
            Some("games".to_owned()),
            0.95,
            "active sway node",
        );

        assert_eq!(snapshot.elapsed_ms, 250);
        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(1234));
        assert_eq!(snapshot.app_id.as_deref(), Some("steam"));
        assert_eq!(snapshot.class.as_deref(), Some("Steam"));
        assert_eq!(snapshot.title, None);
        assert_eq!(snapshot.window_id.as_deref(), Some("42"));
        assert_eq!(snapshot.workspace.as_deref(), Some("games"));
        assert_eq!(snapshot.confidence, 0.95);
        assert_eq!(snapshot.reason, "active sway node");
    }

    #[test]
    fn available_snapshot_keeps_title_when_explicitly_allowed() {
        let snapshot = ForegroundWindowSnapshot::available(
            250,
            ForegroundSource::Hyprland,
            Some(1234),
            Some("firefox".to_owned()),
            Some("firefox".to_owned()),
            Some("Private browser tab".to_owned()),
            true,
            Some("0xabc".to_owned()),
            Some("web".to_owned()),
            0.90,
            "active hyprland client",
        );

        assert_eq!(snapshot.title.as_deref(), Some("Private browser tab"));
    }

    #[test]
    fn event_constructor_redacts_title_by_default() {
        let event = ForegroundEvent::new(
            500,
            ForegroundSource::X11,
            ForegroundProviderStatus::Available,
            Some(5678),
            None,
            Some("Firefox".to_owned()),
            Some("Sensitive tab title".to_owned()),
            false,
            Some("0x1200007".to_owned()),
            Some("1".to_owned()),
            0.80,
            "active x11 window",
        );

        assert_eq!(event.title, None);
        assert_eq!(event.source, ForegroundSource::X11);
        assert_eq!(event.status, ForegroundProviderStatus::Available);
    }

    #[test]
    fn event_from_snapshot_applies_title_policy_again() {
        let snapshot = ForegroundWindowSnapshot {
            elapsed_ms: 1_000,
            source: Some(ForegroundSource::Sway),
            status: ForegroundProviderStatus::Available,
            pid: Some(9000),
            app_id: Some("foot".to_owned()),
            class: None,
            title: Some("terminal: private path".to_owned()),
            window_id: Some("17".to_owned()),
            workspace: Some("dev".to_owned()),
            confidence: 1.0,
            stale_ms: None,
            reason: "test snapshot with title already present".to_owned(),
        };

        let redacted = ForegroundEvent::from_snapshot(&snapshot, false).unwrap();
        let included = ForegroundEvent::from_snapshot(&snapshot, true).unwrap();

        assert_eq!(redacted.title, None);
        assert_eq!(included.title.as_deref(), Some("terminal: private path"));
    }

    #[test]
    fn event_from_snapshot_requires_source() {
        let snapshot = ForegroundWindowSnapshot {
            source: None,
            status: ForegroundProviderStatus::Unavailable,
            reason: "no provider selected".to_owned(),
            ..ForegroundWindowSnapshot::default()
        };

        assert!(ForegroundEvent::from_snapshot(&snapshot, false).is_none());
    }

    #[test]
    fn serde_uses_snake_case_for_enums() {
        let event = ForegroundEvent::new(
            100,
            ForegroundSource::Hyprland,
            ForegroundProviderStatus::Unavailable,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            0.0,
            "hyprctl unavailable",
        );

        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains(r#""source":"hyprland""#));
        assert!(json.contains(r#""status":"unavailable""#));
    }
}
