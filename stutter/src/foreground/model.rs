//! Foreground snapshot and event data models.
//!
//! Owns foreground source/status enums, redacted snapshot/event DTOs, and default foreground sampling
//! constants. Does not own provider process execution, compositor parsing, or stale-snapshot policy.

use serde::{Deserialize, Deserializer, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ForegroundTarget {
    pub pid: Option<u32>,
    pub app_id: Option<String>,
    pub class: Option<String>,
    pub title: Option<String>,
    pub window_id: Option<String>,
    pub workspace: Option<String>,
}

pub type Confidence = f32;

pub const CONFIDENCE_ZERO: Confidence = 0.0;
pub const CONFIDENCE_LOW: Confidence = 0.35;
pub const CONFIDENCE_MEDIUM: Confidence = 0.65;
pub const CONFIDENCE_HIGH: Confidence = 0.95;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForegroundReason {
    pub reason: String,
}

impl ForegroundReason {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RejectedForegroundCandidate {
    pub target: ForegroundTarget,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ForegroundDecision {
    pub target: Option<ForegroundTarget>,
    pub confidence: Confidence,
    pub reasons: Vec<ForegroundReason>,
    #[serde(default)]
    pub rejected_candidates: Vec<RejectedForegroundCandidate>,
}

impl ForegroundDecision {
    pub fn new(
        target: Option<ForegroundTarget>,
        confidence: Confidence,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            target,
            confidence,
            reasons: vec![ForegroundReason::new(reason)],
            rejected_candidates: Vec::new(),
        }
    }

    pub fn primary_reason(&self) -> Option<&str> {
        self.reasons.first().map(|reason| reason.reason.as_str())
    }

    pub fn reason_strings(&self) -> Vec<String> {
        self.reasons
            .iter()
            .map(|reason| reason.reason.clone())
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ForegroundWindowSnapshot {
    pub elapsed_ms: u64,

    pub source: Option<ForegroundSource>,
    pub status: ForegroundProviderStatus,

    pub decision: ForegroundDecision,
    pub stale_ms: Option<u64>,
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
    pub confidence: Confidence,
    pub reason: String,
}

impl ForegroundWindowSnapshot {
    pub fn unsupported(elapsed_ms: u64, reason: impl Into<String>) -> Self {
        Self {
            elapsed_ms,
            source: Some(ForegroundSource::Unsupported),
            status: ForegroundProviderStatus::Unsupported,
            decision: ForegroundDecision::new(None, CONFIDENCE_ZERO, reason),
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
            decision: ForegroundDecision::new(None, CONFIDENCE_ZERO, reason),
            ..Self::default()
        }
    }

    pub fn available(input: ForegroundAvailableInput) -> Self {
        let title = redact_title_unless_allowed(input.title, input.include_title);
        let target = foreground_target_from_parts(
            input.pid,
            input.app_id,
            input.class,
            title,
            input.window_id,
            input.workspace,
        );

        Self {
            elapsed_ms: input.elapsed_ms,
            source: Some(input.source),
            status: ForegroundProviderStatus::Available,
            decision: ForegroundDecision::new(target, input.confidence, input.reason),
            stale_ms: None,
        }
    }

    pub fn with_title_policy(mut self, title: Option<String>, include_title: bool) -> Self {
        if let Some(t) = self.decision.target.as_mut() {
            t.title = redact_title_unless_allowed(title, include_title);
        }
        for rejected in &mut self.decision.rejected_candidates {
            rejected.target.title =
                redact_title_unless_allowed(rejected.target.title.take(), include_title);
        }
        self
    }

    pub fn redact_title(mut self) -> Self {
        if let Some(t) = self.decision.target.as_mut() {
            t.title = None;
        }
        for rejected in &mut self.decision.rejected_candidates {
            rejected.target.title = None;
        }
        self
    }

    pub fn to_event(&self, include_title: bool) -> Option<ForegroundEvent> {
        let source = self.source?;

        let mut decision = self.decision.clone();
        if let Some(t) = decision.target.as_mut() {
            t.title = redact_title_unless_allowed(t.title.take(), include_title);
        }
        for rejected in &mut decision.rejected_candidates {
            rejected.target.title =
                redact_title_unless_allowed(rejected.target.title.take(), include_title);
        }

        Some(ForegroundEvent {
            elapsed_ms: self.elapsed_ms,
            source,
            status: self.status,
            decision,
            stale_ms: self.stale_ms,
        })
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ForegroundEvent {
    pub elapsed_ms: u64,
    pub source: ForegroundSource,
    pub status: ForegroundProviderStatus,
    pub decision: ForegroundDecision,
    #[serde(default)]
    pub stale_ms: Option<u64>,
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
    pub confidence: Confidence,
    pub reason: String,
    pub stale_ms: Option<u64>,
}

impl ForegroundEvent {
    pub fn new(input: ForegroundEventInput) -> Self {
        let title = redact_title_unless_allowed(input.title, input.include_title);
        let target = foreground_target_from_parts(
            input.pid,
            input.app_id,
            input.class,
            title,
            input.window_id,
            input.workspace,
        );

        Self {
            elapsed_ms: input.elapsed_ms,
            source: input.source,
            status: input.status,
            decision: ForegroundDecision::new(target, input.confidence, input.reason),
            stale_ms: input.stale_ms,
        }
    }

    pub fn from_snapshot(snapshot: &ForegroundWindowSnapshot, include_title: bool) -> Option<Self> {
        snapshot.to_event(include_title)
    }

    pub fn redact_title(mut self) -> Self {
        if let Some(t) = self.decision.target.as_mut() {
            t.title = None;
        }
        for rejected in &mut self.decision.rejected_candidates {
            rejected.target.title = None;
        }
        self
    }
}

impl<'de> Deserialize<'de> for ForegroundEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ForegroundEventWire::deserialize(deserializer)?;
        let decision = wire
            .decision
            .filter(decision_has_content)
            .unwrap_or_else(|| {
                ForegroundDecision::new(
                    foreground_target_from_parts(
                        wire.pid,
                        wire.app_id,
                        wire.class,
                        wire.title,
                        wire.window_id,
                        wire.workspace,
                    ),
                    wire.confidence.unwrap_or(CONFIDENCE_ZERO),
                    wire.reason.unwrap_or_default(),
                )
            });

        Ok(Self {
            elapsed_ms: wire.elapsed_ms,
            source: wire.source,
            status: wire.status,
            decision,
            stale_ms: wire.stale_ms,
        })
    }
}

#[derive(Deserialize)]
struct ForegroundEventWire {
    elapsed_ms: u64,
    source: ForegroundSource,
    status: ForegroundProviderStatus,
    #[serde(default)]
    decision: Option<ForegroundDecision>,
    #[serde(default)]
    stale_ms: Option<u64>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    window_id: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    confidence: Option<Confidence>,
    #[serde(default)]
    reason: Option<String>,
}

fn decision_has_content(decision: &ForegroundDecision) -> bool {
    decision.target.is_some()
        || decision.confidence != CONFIDENCE_ZERO
        || !decision.reasons.is_empty()
        || !decision.rejected_candidates.is_empty()
}

pub fn redact_title_unless_allowed(title: Option<String>, include_title: bool) -> Option<String> {
    if include_title { title } else { None }
}

fn foreground_target_from_parts(
    pid: Option<u32>,
    app_id: Option<String>,
    class: Option<String>,
    title: Option<String>,
    window_id: Option<String>,
    workspace: Option<String>,
) -> Option<ForegroundTarget> {
    if pid.is_none()
        && app_id.is_none()
        && class.is_none()
        && title.is_none()
        && window_id.is_none()
        && workspace.is_none()
    {
        return None;
    }

    Some(ForegroundTarget {
        pid,
        app_id,
        class,
        title,
        window_id,
        workspace,
    })
}

pub const DEFAULT_FOREGROUND_MAX_STALE_MS: u64 = 2_500;
pub const DEFAULT_FOREGROUND_INCLUDE_TITLE: bool = false;
