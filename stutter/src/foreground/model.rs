//! Foreground snapshot and event data models.
//!
//! Owns foreground source/status enums, redacted snapshot/event DTOs, and default foreground sampling
//! constants. Does not own provider process execution, compositor parsing, or stale-snapshot policy.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ForegroundSource {
    #[default]
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

#[derive(Debug, Clone)]
pub struct ForegroundAvailableInput {
    pub elapsed_ms: u64,
    pub source: ForegroundSource,
    pub pid: Option<u32>,
    pub app_id: Option<String>,
    pub class: Option<String>,
    pub title: Option<String>,
    pub include_title: bool,
    pub window_id: Option<String>,
    pub workspace: Option<String>,
    pub confidence: f32,
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

    pub fn available(input: ForegroundAvailableInput) -> Self {
        Self {
            elapsed_ms: input.elapsed_ms,
            source: Some(input.source),
            status: ForegroundProviderStatus::Available,
            pid: input.pid,
            app_id: input.app_id,
            class: input.class,
            title: redact_title_unless_allowed(input.title, input.include_title),
            window_id: input.window_id,
            workspace: input.workspace,
            confidence: input.confidence,
            stale_ms: None,
            reason: input.reason,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone)]
pub struct ForegroundEventInput {
    pub elapsed_ms: u64,
    pub source: ForegroundSource,
    pub status: ForegroundProviderStatus,
    pub pid: Option<u32>,
    pub app_id: Option<String>,
    pub class: Option<String>,
    pub title: Option<String>,
    pub include_title: bool,
    pub window_id: Option<String>,
    pub workspace: Option<String>,
    pub confidence: f32,
    pub reason: String,
}

impl ForegroundEvent {
    pub fn new(input: ForegroundEventInput) -> Self {
        Self {
            elapsed_ms: input.elapsed_ms,
            source: input.source,
            status: input.status,
            pid: input.pid,
            app_id: input.app_id,
            class: input.class,
            title: redact_title_unless_allowed(input.title, input.include_title),
            window_id: input.window_id,
            workspace: input.workspace,
            confidence: input.confidence,
            reason: input.reason,
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
