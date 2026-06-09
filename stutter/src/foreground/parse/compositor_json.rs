//! Safe foreground-helper JSON parsing.
//!
//! This parser is intentionally compositor-neutral. GNOME/KDE providers only
//! consume explicit helper output; they do not call unsafe compositor eval APIs.

use serde::Deserialize;

use crate::foreground::model::{
    CONFIDENCE_HIGH, CONFIDENCE_LOW, CONFIDENCE_MEDIUM, CONFIDENCE_ZERO, ForegroundDecision,
    ForegroundProviderStatus, ForegroundReason, ForegroundSource, ForegroundTarget,
    ForegroundWindowSnapshot,
};

#[derive(Debug, Deserialize)]
struct HelperForegroundWindow {
    pid: Option<u32>,
    app_id: Option<String>,
    class: Option<String>,
    title: Option<String>,
    window_id: Option<String>,
    workspace: Option<String>,
    confidence: Option<f32>,
    reason: Option<String>,
}

pub(crate) fn snapshot_from_compositor_helper_json(
    elapsed_ms: u64,
    source: ForegroundSource,
    provider_name: &str,
    json: &str,
) -> ForegroundWindowSnapshot {
    let window = match serde_json::from_str::<HelperForegroundWindow>(json) {
        Ok(window) => window,
        Err(err) => {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(source),
                status: ForegroundProviderStatus::Error,
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason {
                        reason: format!(
                            "failed to parse {provider_name} foreground helper JSON: {err}"
                        ),
                    }],
                    rejected_candidates: Vec::new(),
                },
                stale_ms: None,
            };
        }
    };

    let target = ForegroundTarget {
        pid: window.pid,
        app_id: non_empty(window.app_id),
        class: non_empty(window.class),
        title: non_empty(window.title),
        window_id: non_empty(window.window_id),
        workspace: non_empty(window.workspace),
    };

    if target.pid.is_none()
        && target.app_id.is_none()
        && target.class.is_none()
        && target.window_id.is_none()
    {
        return ForegroundWindowSnapshot {
            elapsed_ms,
            source: Some(source),
            status: ForegroundProviderStatus::Unavailable,
            decision: ForegroundDecision {
                target: None,
                confidence: CONFIDENCE_ZERO,
                reasons: vec![ForegroundReason {
                    reason: format!(
                        "{provider_name} foreground helper did not report an active foreground window identity"
                    ),
                }],
                rejected_candidates: Vec::new(),
            },
            stale_ms: None,
        };
    }

    let confidence = normalized_confidence(window.confidence, &target);
    let reason = window
        .reason
        .and_then(|value| non_empty(Some(value)))
        .unwrap_or_else(|| format!("active {provider_name} foreground window from safe helper"));

    ForegroundWindowSnapshot {
        elapsed_ms,
        source: Some(source),
        status: ForegroundProviderStatus::Available,
        decision: ForegroundDecision {
            target: Some(target),
            confidence,
            reasons: vec![ForegroundReason { reason }],
            rejected_candidates: Vec::new(),
        },
        stale_ms: None,
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn normalized_confidence(value: Option<f32>, target: &ForegroundTarget) -> f32 {
    if let Some(value) = value
        && value.is_finite()
    {
        return value.clamp(CONFIDENCE_ZERO, CONFIDENCE_HIGH);
    }

    if target.pid.is_some() && (target.app_id.is_some() || target.class.is_some()) {
        CONFIDENCE_HIGH
    } else if target.pid.is_some() || target.app_id.is_some() || target.class.is_some() {
        CONFIDENCE_MEDIUM
    } else {
        CONFIDENCE_LOW
    }
}
