use std::process::Command;

use serde::{Deserialize, Serialize};

pub(crate) mod command;
pub(crate) mod model;
pub(crate) mod parse;
pub(crate) mod provider;
pub(crate) mod providers;
pub(crate) mod resolver;

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

pub fn auto_foreground_provider() -> Box<dyn ForegroundProvider + Send> {
    if SwayForegroundProvider::is_detected() {
        return Box::new(SwayForegroundProvider::new());
    }

    if HyprlandForegroundProvider::is_detected() {
        return Box::new(HyprlandForegroundProvider::new());
    }

    if is_generic_wayland_without_supported_foreground_api() {
        if current_desktop_looks_like_gnome_or_kde() {
            return Box::new(UnsupportedForegroundProvider::new(
                "GNOME/KDE Wayland session detected, but no safe generic Wayland foreground-window API is available",
            ));
        }
        return Box::new(UnsupportedForegroundProvider::generic_wayland());
    }

    if X11ForegroundProvider::is_detected() {
        return Box::new(X11ForegroundProvider::new());
    }

    Box::new(UnsupportedForegroundProvider::new(
        "no supported foreground-window provider detected",
    ))
}

pub fn auto_foreground_resolver() -> ForegroundResolver {
    ForegroundResolver::new(auto_foreground_provider())
}

fn is_generic_wayland_without_supported_foreground_api() -> bool {
    if std::env::var("WAYLAND_DISPLAY").is_err() {
        return false;
    }

    if std::env::var("SWAYSOCK").is_ok() {
        return false;
    }

    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
        return false;
    }

    true
}

fn current_desktop_looks_like_gnome_or_kde() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("DESKTOP_SESSION"))
        .unwrap_or_default()
        .to_ascii_lowercase();

    desktop.contains("gnome") || desktop.contains("kde") || desktop.contains("plasma")
}

#[derive(Debug, Deserialize)]
struct HyprlandActiveWindow {
    address: Option<String>,
    class: Option<String>,
    #[serde(rename = "initialClass")]
    initial_class: Option<String>,
    title: Option<String>,
    pid: Option<u32>,
    workspace: Option<HyprlandWorkspace>,
}

#[derive(Debug, Deserialize)]
struct HyprlandWorkspace {
    name: Option<String>,
}

pub(crate) fn hyprland_snapshot_from_activewindow_json(
    elapsed_ms: u64,
    active_window_json: &str,
) -> ForegroundWindowSnapshot {
    let active_window = match serde_json::from_str::<HyprlandActiveWindow>(active_window_json) {
        Ok(active_window) => active_window,
        Err(err) => {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!("failed to parse hyprctl activewindow JSON: {err}"),
                ..ForegroundWindowSnapshot::default()
            };
        }
    };

    let class = active_window.class.or(active_window.initial_class);
    let confidence = if active_window.pid.is_some() {
        0.95
    } else if class.is_some() {
        0.65
    } else if active_window.title.is_some() || active_window.address.is_some() {
        0.35
    } else {
        0.0
    };

    ForegroundWindowSnapshot {
        elapsed_ms,
        source: Some(ForegroundSource::Hyprland),
        status: ForegroundProviderStatus::Available,
        pid: active_window.pid,
        app_id: None,
        class,
        title: active_window.title,
        window_id: active_window.address,
        workspace: active_window.workspace.and_then(|workspace| workspace.name),
        confidence,
        stale_ms: None,
        reason: "active Hyprland window from hyprctl activewindow".to_owned(),
    }
}

#[derive(Debug, Clone)]
pub struct HyprlandForegroundProvider {
    hyprctl: String,
}

impl Default for HyprlandForegroundProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl HyprlandForegroundProvider {
    pub fn new() -> Self {
        Self {
            hyprctl: "hyprctl".to_owned(),
        }
    }

    #[cfg(test)]
    pub fn with_hyprctl(mut self, hyprctl: impl Into<String>) -> Self {
        self.hyprctl = hyprctl.into();
        self
    }

    pub fn is_detected() -> bool {
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
    }
}

impl ForegroundProvider for HyprlandForegroundProvider {
    fn source(&self) -> ForegroundSource {
        ForegroundSource::Hyprland
    }

    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
        if !Self::is_detected() {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Unavailable,
                confidence: 0.0,
                reason: "HYPRLAND_INSTANCE_SIGNATURE is not set; Hyprland foreground provider is unavailable".to_owned(),
                ..ForegroundWindowSnapshot::default()
            };
        }

        let output = match Command::new(&self.hyprctl)
            .args(["activewindow", "-j"])
            .output()
        {
            Ok(output) => output,
            Err(err) => {
                return ForegroundWindowSnapshot {
                    elapsed_ms,
                    source: Some(ForegroundSource::Hyprland),
                    status: ForegroundProviderStatus::Error,
                    confidence: 0.0,
                    reason: format!("failed to run {} activewindow -j: {err}", self.hyprctl),
                    ..ForegroundWindowSnapshot::default()
                };
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!(
                    "{} activewindow -j exited with status {}; stderr={}",
                    self.hyprctl,
                    output.status,
                    stderr.trim()
                ),
                ..ForegroundWindowSnapshot::default()
            };
        }

        match String::from_utf8(output.stdout) {
            Ok(stdout) => hyprland_snapshot_from_activewindow_json(elapsed_ms, &stdout),
            Err(err) => ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!("hyprctl activewindow JSON output was not valid UTF-8: {err}"),
                ..ForegroundWindowSnapshot::default()
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct SwayForegroundProvider {
    swaymsg: String,
}

impl Default for SwayForegroundProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SwayForegroundProvider {
    pub fn new() -> Self {
        Self {
            swaymsg: String::new(),
        }
        .with_swaymsg("swaymsg")
    }

    pub fn with_swaymsg(mut self, swaymsg: impl Into<String>) -> Self {
        self.swaymsg = swaymsg.into();
        self
    }

    pub fn is_detected() -> bool {
        std::env::var("SWAYSOCK").is_ok()
    }

    pub fn sample_from_tree_json(
        &self,
        elapsed_ms: u64,
        tree_json: &str,
    ) -> ForegroundWindowSnapshot {
        match serde_json::from_str::<SwayNode>(tree_json) {
            Ok(root) => focused_sway_snapshot_from_tree(elapsed_ms, &root),
            Err(err) => ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!("failed to parse swaymsg get_tree JSON: {err}"),
                ..ForegroundWindowSnapshot::default()
            },
        }
    }
}

impl ForegroundProvider for SwayForegroundProvider {
    fn source(&self) -> ForegroundSource {
        ForegroundSource::Sway
    }

    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
        if !Self::is_detected() {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Unavailable,
                confidence: 0.0,
                reason: "SWAYSOCK is not set; Sway foreground provider is unavailable".to_owned(),
                ..ForegroundWindowSnapshot::default()
            };
        }

        let output = match Command::new(&self.swaymsg)
            .args(["-t", "get_tree", "-r"])
            .output()
        {
            Ok(output) => output,
            Err(err) => {
                return ForegroundWindowSnapshot {
                    elapsed_ms,
                    source: Some(ForegroundSource::Sway),
                    status: ForegroundProviderStatus::Error,
                    confidence: 0.0,
                    reason: format!("failed to run {} -t get_tree -r: {err}", self.swaymsg),
                    ..ForegroundWindowSnapshot::default()
                };
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!(
                    "{} -t get_tree -r exited with status {}; stderr={}",
                    self.swaymsg,
                    output.status,
                    stderr.trim()
                ),
                ..ForegroundWindowSnapshot::default()
            };
        }

        match String::from_utf8(output.stdout) {
            Ok(stdout) => self.sample_from_tree_json(elapsed_ms, &stdout),
            Err(err) => ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!("swaymsg get_tree output was not valid UTF-8: {err}"),
                ..ForegroundWindowSnapshot::default()
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct SwayNode {
    id: Option<i64>,
    name: Option<String>,
    focused: Option<bool>,
    pid: Option<u32>,
    app_id: Option<String>,
    window: Option<i64>,
    window_properties: Option<SwayWindowProperties>,
    nodes: Option<Vec<SwayNode>>,
    floating_nodes: Option<Vec<SwayNode>>,
    #[serde(rename = "type")]
    node_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SwayWindowProperties {
    class: Option<String>,
    instance: Option<String>,
    title: Option<String>,
}

fn focused_sway_snapshot_from_tree(elapsed_ms: u64, root: &SwayNode) -> ForegroundWindowSnapshot {
    let Some((focused, workspace)) = find_focused_sway_node(root, None) else {
        return ForegroundWindowSnapshot {
            elapsed_ms,
            source: Some(ForegroundSource::Sway),
            status: ForegroundProviderStatus::Unavailable,
            confidence: 0.0,
            reason: "sway tree did not contain a focused node".to_owned(),
            ..ForegroundWindowSnapshot::default()
        };
    };

    let class = focused
        .window_properties
        .as_ref()
        .and_then(|properties| properties.class.clone())
        .or_else(|| {
            focused
                .window_properties
                .as_ref()
                .and_then(|properties| properties.instance.clone())
        });
    let title = focused
        .window_properties
        .as_ref()
        .and_then(|properties| properties.title.clone())
        .or_else(|| focused.name.clone());
    let window_id = focused
        .window
        .map(|window| window.to_string())
        .or_else(|| focused.id.map(|id| id.to_string()));
    let confidence = sway_confidence(focused);

    ForegroundWindowSnapshot {
        elapsed_ms,
        source: Some(ForegroundSource::Sway),
        status: ForegroundProviderStatus::Available,
        pid: focused.pid,
        app_id: focused.app_id.clone(),
        class,
        title,
        window_id,
        workspace: workspace.cloned(),
        confidence,
        stale_ms: None,
        reason: "focused Sway node from swaymsg get_tree".to_owned(),
    }
}

fn find_focused_sway_node<'a>(
    node: &'a SwayNode,
    workspace: Option<&'a String>,
) -> Option<(&'a SwayNode, Option<&'a String>)> {
    let current_workspace = if node.node_type.as_deref() == Some("workspace") {
        node.name.as_ref()
    } else {
        workspace
    };

    if node.focused.unwrap_or(false) {
        return Some((node, current_workspace));
    }

    for child in node
        .nodes
        .iter()
        .flatten()
        .chain(node.floating_nodes.iter().flatten())
    {
        if let Some(found) = find_focused_sway_node(child, current_workspace) {
            return Some(found);
        }
    }

    None
}

fn sway_confidence(node: &SwayNode) -> f32 {
    if node.pid.is_some() {
        0.95
    } else if node.app_id.is_some()
        || node
            .window_properties
            .as_ref()
            .is_some_and(|properties| properties.class.is_some())
    {
        0.65
    } else if node.name.is_some() || node.window.is_some() || node.id.is_some() {
        0.35
    } else {
        0.0
    }
}

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
        std::env::var("DISPLAY").is_ok() && which::which("xprop").is_ok()
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

        if which::which(&self.xprop).is_err() {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::X11),
                status: ForegroundProviderStatus::Unavailable,
                confidence: 0.0,
                reason: format!(
                    "{} was not found in PATH; X11 foreground provider is unavailable",
                    self.xprop
                ),
                ..ForegroundWindowSnapshot::default()
            };
        }

        let active_output = match Command::new(&self.xprop)
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
                        self.xprop
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
                    self.xprop,
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

        let properties_output = match Command::new(&self.xprop)
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
                        self.xprop, window_id
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
                    self.xprop,
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct X11WindowProperties {
    pid: Option<u32>,
    instance: Option<String>,
    class: Option<String>,
    net_wm_name: Option<String>,
    wm_name: Option<String>,
}

fn parse_x11_active_window_id(output: &str) -> Option<String> {
    for line in output.lines() {
        if !line.contains("_NET_ACTIVE_WINDOW") {
            continue;
        }

        let Some((_, value)) = line.split_once('#') else {
            continue;
        };

        let value = value.trim().trim_end_matches(',');
        if value.is_empty() || value == "0x0" || value.eq_ignore_ascii_case("none") {
            return None;
        }

        return Some(value.to_owned());
    }

    None
}

fn parse_x11_window_properties(output: &str) -> X11WindowProperties {
    let mut properties = X11WindowProperties::default();

    for line in output.lines() {
        if line.starts_with("_NET_WM_PID") {
            properties.pid = parse_x11_u32_value(line);
        } else if line.starts_with("WM_CLASS") {
            let values = parse_x11_quoted_strings(line);
            properties.instance = values.first().cloned();
            properties.class = values.get(1).cloned().or_else(|| values.first().cloned());
        } else if line.starts_with("_NET_WM_NAME") {
            properties.net_wm_name = parse_x11_string_value(line);
        } else if line.starts_with("WM_NAME") {
            properties.wm_name = parse_x11_string_value(line);
        }
    }

    properties
}

fn parse_x11_u32_value(line: &str) -> Option<u32> {
    let (_, value) = line.split_once('=')?;
    value.trim().parse::<u32>().ok()
}

fn parse_x11_string_value(line: &str) -> Option<String> {
    let values = parse_x11_quoted_strings(line);
    if let Some(value) = values.first() {
        return Some(value.clone());
    }

    let (_, value) = line.split_once('=')?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn parse_x11_quoted_strings(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '"' {
            continue;
        }

        let mut value = String::new();
        let mut escaped = false;

        for inner in chars.by_ref() {
            if escaped {
                value.push(inner);
                escaped = false;
                continue;
            }

            if inner == '\\' {
                escaped = true;
                continue;
            }

            if inner == '"' {
                break;
            }

            value.push(inner);
        }

        values.push(value);
    }

    values
}

fn x11_confidence(properties: &X11WindowProperties, window_id: &str) -> f32 {
    if properties.pid.is_some() {
        0.90
    } else if properties.class.is_some() || properties.instance.is_some() {
        0.55
    } else if properties.net_wm_name.is_some()
        || properties.wm_name.is_some()
        || !window_id.is_empty()
    {
        0.35
    } else {
        0.0
    }
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
#[path = "foreground/tests/mod.rs"]
mod tests;
