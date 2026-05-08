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

#[cfg(test)]
mod tests {
    use super::*;

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
