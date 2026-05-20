//! Foreground provider trait and unsupported-provider implementation.
//!
//! Owns the provider abstraction used by the resolver plus the explicit unsupported provider. Does
//! not own compositor-specific command execution or parser logic.

use super::model::{ForegroundProviderStatus, ForegroundSource, ForegroundWindowSnapshot};

pub trait ForegroundProvider {
    fn source(&self) -> ForegroundSource;
    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot;
}

pub const GENERIC_WAYLAND_UNSUPPORTED_REASON: &str =
    "no safe generic Wayland foreground-window API detected";

#[derive(Debug, Clone)]
pub struct UnsupportedForegroundProvider {
    reason: String,
}

impl Default for UnsupportedForegroundProvider {
    fn default() -> Self {
        Self::generic_wayland()
    }
}

impl UnsupportedForegroundProvider {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    pub fn generic_wayland() -> Self {
        Self::new(GENERIC_WAYLAND_UNSUPPORTED_REASON)
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl ForegroundProvider for UnsupportedForegroundProvider {
    fn source(&self) -> ForegroundSource {
        ForegroundSource::Unsupported
    }

    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
        ForegroundWindowSnapshot {
            elapsed_ms,
            source: Some(ForegroundSource::Unsupported),
            status: ForegroundProviderStatus::Unsupported,
            confidence: 0.0,
            reason: self.reason().to_owned(),
            ..ForegroundWindowSnapshot::default()
        }
    }
}
