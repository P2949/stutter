//! X11 foreground provider.
//!
//! Owns `xprop` process execution and X11 active-window snapshot construction. Does not own parser
//! tokenization details or resolver stale-snapshot policy.

use std::process::Command;

use crate::foreground::{
    command::resolve_trusted_foreground_helper,
    model::{ForegroundProviderStatus, ForegroundSource, ForegroundWindowSnapshot},
    parse::x11::{parse_x11_active_window_id, parse_x11_window_properties, x11_confidence},
    provider::ForegroundProvider,
};

#[derive(Debug, Clone)]
pub struct X11ForegroundProvider {
    xprop: String,
}

impl Default for X11ForegroundProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl X11ForegroundProvider {
    pub fn new() -> Self {
        Self {
            xprop: String::new(),
        }
        .with_xprop("xprop")
    }

    pub fn with_xprop(mut self, xprop: impl Into<String>) -> Self {
        self.xprop = xprop.into();
        self
    }

    pub fn is_detected() -> bool {
        std::env::var("DISPLAY").is_ok() && resolve_trusted_foreground_helper("xprop").is_some()
    }

    pub fn sample_from_xprop_outputs(
        &self,
        elapsed_ms: u64,
        active_window_output: &str,
        window_properties_output: &str,
    ) -> ForegroundWindowSnapshot {
        let Some(window_id) = parse_x11_active_window_id(active_window_output) else {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::X11),
                status: ForegroundProviderStatus::Unavailable,
                confidence: 0.0,
                reason: "xprop root output did not contain an active X11 window".to_owned(),
                ..ForegroundWindowSnapshot::default()
            };
        };

        let properties = parse_x11_window_properties(window_properties_output);
        let confidence = x11_confidence(&properties, &window_id);

        ForegroundWindowSnapshot {
            elapsed_ms,
            source: Some(ForegroundSource::X11),
            status: ForegroundProviderStatus::Available,
            pid: properties.pid,
            app_id: properties.instance,
            class: properties.class,
            title: properties.net_wm_name.or(properties.wm_name),
            window_id: Some(window_id),
            workspace: None,
            confidence,
            stale_ms: None,
            reason: "active X11 window from xprop".to_owned(),
        }
    }
}

impl ForegroundProvider for X11ForegroundProvider {
    fn source(&self) -> ForegroundSource {
        ForegroundSource::X11
    }

    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
        if std::env::var("DISPLAY").is_err() {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::X11),
                status: ForegroundProviderStatus::Unavailable,
                confidence: 0.0,
                reason: "DISPLAY is not set; X11 foreground provider is unavailable".to_owned(),
                ..ForegroundWindowSnapshot::default()
            };
        }

        let Some(xprop) = resolve_trusted_foreground_helper(&self.xprop) else {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::X11),
                status: ForegroundProviderStatus::Unavailable,
                confidence: 0.0,
                reason: format!(
                    "{} was not found in trusted foreground helper paths; X11 foreground provider is unavailable",
                    self.xprop
                ),
                ..ForegroundWindowSnapshot::default()
            };
        };

        let active_output = match Command::new(&xprop)
            .args(["-root", "_NET_ACTIVE_WINDOW"])
            .output()
        {
            Ok(output) => output,
            Err(err) => {
                return ForegroundWindowSnapshot {
                    elapsed_ms,
                    source: Some(ForegroundSource::X11),
                    status: ForegroundProviderStatus::Error,
                    confidence: 0.0,
                    reason: format!(
                        "failed to run {} -root _NET_ACTIVE_WINDOW: {err}",
                        xprop.display()
                    ),
                    ..ForegroundWindowSnapshot::default()
                };
            }
        };

        if !active_output.status.success() {
            let stderr = String::from_utf8_lossy(&active_output.stderr);
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::X11),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!(
                    "{} -root _NET_ACTIVE_WINDOW exited with status {}; stderr={}",
                    xprop.display(),
                    active_output.status,
                    stderr.trim()
                ),
                ..ForegroundWindowSnapshot::default()
            };
        }

        let active_stdout = match String::from_utf8(active_output.stdout) {
            Ok(stdout) => stdout,
            Err(err) => {
                return ForegroundWindowSnapshot {
                    elapsed_ms,
                    source: Some(ForegroundSource::X11),
                    status: ForegroundProviderStatus::Error,
                    confidence: 0.0,
                    reason: format!("xprop _NET_ACTIVE_WINDOW output was not valid UTF-8: {err}"),
                    ..ForegroundWindowSnapshot::default()
                };
            }
        };

        let Some(window_id) = parse_x11_active_window_id(&active_stdout) else {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::X11),
                status: ForegroundProviderStatus::Unavailable,
                confidence: 0.0,
                reason: "xprop root output did not contain an active X11 window".to_owned(),
                ..ForegroundWindowSnapshot::default()
            };
        };

        let properties_output = match Command::new(&xprop)
            .args([
                "-id",
                &window_id,
                "_NET_WM_PID",
                "WM_CLASS",
                "_NET_WM_NAME",
                "WM_NAME",
            ])
            .output()
        {
            Ok(output) => output,
            Err(err) => {
                return ForegroundWindowSnapshot {
                    elapsed_ms,
                    source: Some(ForegroundSource::X11),
                    status: ForegroundProviderStatus::Error,
                    confidence: 0.0,
                    reason: format!(
                        "failed to run {} -id {} _NET_WM_PID WM_CLASS _NET_WM_NAME WM_NAME: {err}",
                        xprop.display(),
                        window_id
                    ),
                    ..ForegroundWindowSnapshot::default()
                };
            }
        };

        if !properties_output.status.success() {
            let stderr = String::from_utf8_lossy(&properties_output.stderr);
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::X11),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!(
                    "{} -id {} _NET_WM_PID WM_CLASS _NET_WM_NAME WM_NAME exited with status {}; stderr={}",
                    xprop.display(),
                    window_id,
                    properties_output.status,
                    stderr.trim()
                ),
                ..ForegroundWindowSnapshot::default()
            };
        }

        match String::from_utf8(properties_output.stdout) {
            Ok(stdout) => self.sample_from_xprop_outputs(elapsed_ms, &active_stdout, &stdout),
            Err(err) => ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::X11),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!("xprop active window properties output was not valid UTF-8: {err}"),
                ..ForegroundWindowSnapshot::default()
            },
        }
    }
}
