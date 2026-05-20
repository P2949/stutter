//! Foreground resolver stale-snapshot and title-redaction policy.
//!
//! Owns the resolver that wraps providers, applies title policy, and reuses recent good snapshots
//! when providers temporarily fail. Does not own provider selection or compositor parsing.

use crate::foreground::{
    model::{
        DEFAULT_FOREGROUND_INCLUDE_TITLE, DEFAULT_FOREGROUND_MAX_STALE_MS,
        DEFAULT_FOREGROUND_MIN_CONFIDENCE, ForegroundProviderStatus, ForegroundSource,
        ForegroundWindowSnapshot, redact_title_unless_allowed,
    },
    provider::ForegroundProvider,
};

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
