#![allow(dead_code)]

use std::process::Command;

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
            reason: self.reason.clone(),
            ..ForegroundWindowSnapshot::default()
        }
    }
}

pub fn auto_foreground_provider() -> Box<dyn ForegroundProvider + Send> {
    if SwayForegroundProvider::is_detected() {
        return Box::new(SwayForegroundProvider::new());
    }

    if is_generic_wayland_without_supported_foreground_api() {
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
            swaymsg: "swaymsg".to_owned(),
        }
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
            xprop: "xprop".to_owned(),
        }
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
mod tests {
    use super::*;

    #[test]
    fn sway_provider_detection_uses_swaysock_environment() {
        let previous = std::env::var_os("SWAYSOCK");

        unsafe {
            std::env::remove_var("SWAYSOCK");
        }
        assert!(!SwayForegroundProvider::is_detected());

        unsafe {
            std::env::set_var("SWAYSOCK", "/tmp/sway-ipc.sock");
        }
        assert!(SwayForegroundProvider::is_detected());

        unsafe {
            if let Some(previous) = previous {
                std::env::set_var("SWAYSOCK", previous);
            } else {
                std::env::remove_var("SWAYSOCK");
            }
        }
    }

    #[test]
    fn sway_tree_parser_finds_focused_tiled_node_with_pid() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "games",
              "type": "workspace",
              "focused": false,
              "nodes": [
                {
                  "id": 3,
                  "name": "Steam - private title",
                  "focused": true,
                  "pid": 4242,
                  "app_id": "steam",
                  "window": 12345,
                  "window_properties": {
                    "class": "Steam",
                    "instance": "steam",
                    "title": "Steam - private title"
                  },
                  "nodes": [],
                  "floating_nodes": []
                }
              ],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(2_000, json);

        assert_eq!(snapshot.elapsed_ms, 2_000);
        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(4242));
        assert_eq!(snapshot.app_id.as_deref(), Some("steam"));
        assert_eq!(snapshot.class.as_deref(), Some("Steam"));
        assert_eq!(snapshot.title.as_deref(), Some("Steam - private title"));
        assert_eq!(snapshot.window_id.as_deref(), Some("12345"));
        assert_eq!(snapshot.workspace.as_deref(), Some("games"));
        assert_eq!(snapshot.confidence, 0.95);
    }

    #[test]
    fn sway_tree_parser_finds_focused_floating_node() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "dev",
              "type": "workspace",
              "focused": false,
              "nodes": [],
              "floating_nodes": [
                {
                  "id": 9,
                  "name": "floating terminal",
                  "focused": true,
                  "pid": 7777,
                  "app_id": "foot",
                  "window": null,
                  "window_properties": {
                    "class": "foot",
                    "instance": "foot",
                    "title": "floating terminal"
                  },
                  "nodes": [],
                  "floating_nodes": []
                }
              ]
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(3_000, json);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(7777));
        assert_eq!(snapshot.app_id.as_deref(), Some("foot"));
        assert_eq!(snapshot.window_id.as_deref(), Some("9"));
        assert_eq!(snapshot.workspace.as_deref(), Some("dev"));
        assert_eq!(snapshot.confidence, 0.95);
    }

    #[test]
    fn sway_tree_parser_uses_medium_confidence_without_pid() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "web",
              "type": "workspace",
              "focused": false,
              "nodes": [
                {
                  "id": 3,
                  "name": "Firefox",
                  "focused": true,
                  "pid": null,
                  "app_id": "firefox",
                  "window": null,
                  "window_properties": {
                    "class": "Firefox",
                    "instance": "Navigator",
                    "title": "Private tab title"
                  },
                  "nodes": [],
                  "floating_nodes": []
                }
              ],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(4_000, json);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id.as_deref(), Some("firefox"));
        assert_eq!(snapshot.class.as_deref(), Some("Firefox"));
        assert_eq!(snapshot.confidence, 0.65);
    }

    #[test]
    fn sway_tree_parser_uses_low_confidence_for_title_or_window_only() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "misc",
              "type": "workspace",
              "focused": false,
              "nodes": [
                {
                  "id": 3,
                  "name": "unknown window",
                  "focused": true,
                  "pid": null,
                  "app_id": null,
                  "window": 9988,
                  "window_properties": null,
                  "nodes": [],
                  "floating_nodes": []
                }
              ],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(5_000, json);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id, None);
        assert_eq!(snapshot.class, None);
        assert_eq!(snapshot.title.as_deref(), Some("unknown window"));
        assert_eq!(snapshot.window_id.as_deref(), Some("9988"));
        assert_eq!(snapshot.confidence, 0.35);
    }

    #[test]
    fn sway_tree_parser_reports_unavailable_when_no_focused_node_exists() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(6_000, json);

        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        assert_eq!(snapshot.confidence, 0.0);
        assert!(snapshot.reason.contains("did not contain a focused node"));
    }

    #[test]
    fn sway_tree_parser_reports_error_for_invalid_json() {
        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(7_000, "{not-json}");

        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Error);
        assert_eq!(snapshot.confidence, 0.0);
        assert!(
            snapshot
                .reason
                .contains("failed to parse swaymsg get_tree JSON")
        );
    }

    #[test]
    fn sway_provider_titles_are_redacted_by_resolver_default() {
        let provider = SequenceProvider::new(
            ForegroundSource::Sway,
            vec![ForegroundWindowSnapshot {
                elapsed_ms: 0,
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Available,
                pid: Some(4242),
                app_id: Some("steam".to_owned()),
                class: Some("Steam".to_owned()),
                title: Some("Sensitive foreground title".to_owned()),
                window_id: Some("12345".to_owned()),
                workspace: Some("games".to_owned()),
                confidence: 0.95,
                stale_ms: None,
                reason: "test sway provider snapshot".to_owned(),
            }],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider));

        let snapshot = resolver.sample(8_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.title, None);
    }

    #[test]
    fn x11_provider_reports_unavailable_without_display() {
        let previous = std::env::var_os("DISPLAY");

        unsafe {
            std::env::remove_var("DISPLAY");
        }

        let mut provider = X11ForegroundProvider::new();
        let snapshot = provider.sample(1_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::X11));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        assert_eq!(snapshot.confidence, 0.0);
        assert!(snapshot.reason.contains("DISPLAY is not set"));

        unsafe {
            if let Some(previous) = previous {
                std::env::set_var("DISPLAY", previous);
            } else {
                std::env::remove_var("DISPLAY");
            }
        }
    }

    #[test]
    fn x11_provider_checks_xprop_before_sampling() {
        let previous = std::env::var_os("DISPLAY");

        unsafe {
            std::env::set_var("DISPLAY", ":99");
        }

        let mut provider =
            X11ForegroundProvider::new().with_xprop("stutter-definitely-missing-xprop-binary");
        let snapshot = provider.sample(1_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::X11));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        assert_eq!(snapshot.confidence, 0.0);
        assert!(
            snapshot
                .reason
                .contains("stutter-definitely-missing-xprop-binary")
        );
        assert!(snapshot.reason.contains("was not found in PATH"));

        unsafe {
            if let Some(previous) = previous {
                std::env::set_var("DISPLAY", previous);
            } else {
                std::env::remove_var("DISPLAY");
            }
        }
    }

    #[test]
    fn x11_parser_extracts_pid_class_and_title_with_high_confidence() {
        let active = "_NET_ACTIVE_WINDOW(WINDOW): window id # 0x4600007\n";
        let props = r#"
_NET_WM_PID(CARDINAL) = 12345
WM_CLASS(STRING) = "steam_app_379430", "steam_app_379430"
_NET_WM_NAME(UTF8_STRING) = "Kingdom Come: Deliverance"
WM_NAME(STRING) = "fallback title"
"#;

        let provider = X11ForegroundProvider::new();
        let snapshot = provider.sample_from_xprop_outputs(2_000, active, props);

        assert_eq!(snapshot.elapsed_ms, 2_000);
        assert_eq!(snapshot.source, Some(ForegroundSource::X11));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(12345));
        assert_eq!(snapshot.app_id.as_deref(), Some("steam_app_379430"));
        assert_eq!(snapshot.class.as_deref(), Some("steam_app_379430"));
        assert_eq!(snapshot.title.as_deref(), Some("Kingdom Come: Deliverance"));
        assert_eq!(snapshot.window_id.as_deref(), Some("0x4600007"));
        assert_eq!(snapshot.workspace, None);
        assert_eq!(snapshot.confidence, 0.90);
        assert_eq!(snapshot.reason, "active X11 window from xprop");
    }

    #[test]
    fn x11_parser_uses_wm_name_fallback_when_net_wm_name_missing() {
        let active = "_NET_ACTIVE_WINDOW(WINDOW): window id # 0x1200007\n";
        let props = r#"
WM_CLASS(STRING) = "foot", "foot"
WM_NAME(STRING) = "terminal title"
"#;

        let provider = X11ForegroundProvider::new();
        let snapshot = provider.sample_from_xprop_outputs(3_000, active, props);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id.as_deref(), Some("foot"));
        assert_eq!(snapshot.class.as_deref(), Some("foot"));
        assert_eq!(snapshot.title.as_deref(), Some("terminal title"));
        assert_eq!(snapshot.confidence, 0.55);
    }

    #[test]
    fn x11_parser_uses_medium_confidence_for_wm_class_without_pid() {
        let active = "_NET_ACTIVE_WINDOW(WINDOW): window id # 0x4600007\n";
        let props = r#"
WM_CLASS(STRING) = "Navigator", "Firefox"
_NET_WM_NAME(UTF8_STRING) = "Private browser tab"
"#;

        let provider = X11ForegroundProvider::new();
        let snapshot = provider.sample_from_xprop_outputs(4_000, active, props);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id.as_deref(), Some("Navigator"));
        assert_eq!(snapshot.class.as_deref(), Some("Firefox"));
        assert_eq!(snapshot.title.as_deref(), Some("Private browser tab"));
        assert_eq!(snapshot.confidence, 0.55);
    }

    #[test]
    fn x11_parser_reports_unavailable_for_missing_active_window() {
        let provider = X11ForegroundProvider::new();

        for active in [
            "_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0\n",
            "_NET_ACTIVE_WINDOW:  not found.\n",
            "unrelated xprop output\n",
        ] {
            let snapshot = provider.sample_from_xprop_outputs(
                5_000,
                active,
                "WM_CLASS(STRING) = \"steam\", \"Steam\"\n",
            );

            assert_eq!(snapshot.source, Some(ForegroundSource::X11));
            assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
            assert_eq!(snapshot.confidence, 0.0);
            assert!(
                snapshot
                    .reason
                    .contains("did not contain an active X11 window")
            );
        }
    }

    #[test]
    fn x11_parser_handles_escaped_quoted_strings() {
        let values = parse_x11_quoted_strings(r#"WM_CLASS(STRING) = "term\"inal", "Class\\Name""#);

        assert_eq!(
            values,
            vec!["term\"inal".to_owned(), "Class\\Name".to_owned()]
        );
    }

    #[test]
    fn x11_provider_titles_are_redacted_by_resolver_default() {
        let provider = SequenceProvider::new(
            ForegroundSource::X11,
            vec![ForegroundWindowSnapshot {
                elapsed_ms: 0,
                source: Some(ForegroundSource::X11),
                status: ForegroundProviderStatus::Available,
                pid: Some(12345),
                app_id: Some("steam_app_379430".to_owned()),
                class: Some("steam_app_379430".to_owned()),
                title: Some("Kingdom Come: Deliverance".to_owned()),
                window_id: Some("0x4600007".to_owned()),
                workspace: None,
                confidence: 0.90,
                stale_ms: None,
                reason: "test x11 provider snapshot".to_owned(),
            }],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider));

        let snapshot = resolver.sample(6_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::X11));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.title, None);
    }

    #[test]
    fn unsupported_provider_reports_generic_wayland_reason() {
        let mut provider = UnsupportedForegroundProvider::generic_wayland();

        let snapshot = provider.sample(1_234);

        assert_eq!(snapshot.elapsed_ms, 1_234);
        assert_eq!(snapshot.source, Some(ForegroundSource::Unsupported));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unsupported);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id, None);
        assert_eq!(snapshot.class, None);
        assert_eq!(snapshot.title, None);
        assert_eq!(snapshot.window_id, None);
        assert_eq!(snapshot.workspace, None);
        assert_eq!(snapshot.confidence, 0.0);
        assert_eq!(snapshot.reason, GENERIC_WAYLAND_UNSUPPORTED_REASON);
    }

    #[test]
    fn generic_wayland_without_sway_or_hyprland_is_unsupported() {
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        let previous_display = std::env::var_os("DISPLAY");
        let previous_desktop = std::env::var_os("XDG_CURRENT_DESKTOP");

        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
            std::env::remove_var("SWAYSOCK");
            std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
            std::env::set_var("DISPLAY", ":0");
            std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
        }

        assert!(is_generic_wayland_without_supported_foreground_api());
        assert!(current_desktop_looks_like_gnome_or_kde());

        let mut provider = auto_foreground_provider();
        let snapshot = provider.sample(2_000);

        assert_eq!(provider.source(), ForegroundSource::Unsupported);
        assert_eq!(snapshot.source, Some(ForegroundSource::Unsupported));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unsupported);
        assert_eq!(snapshot.reason, GENERIC_WAYLAND_UNSUPPORTED_REASON);
        assert_eq!(snapshot.title, None);

        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
            restore_env_var("DISPLAY", previous_display);
            restore_env_var("XDG_CURRENT_DESKTOP", previous_desktop);
        }
    }

    #[test]
    fn kde_wayland_without_compositor_specific_provider_is_unsupported() {
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        let previous_display = std::env::var_os("DISPLAY");
        let previous_desktop = std::env::var_os("XDG_CURRENT_DESKTOP");

        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
            std::env::remove_var("SWAYSOCK");
            std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
            std::env::set_var("DISPLAY", ":0");
            std::env::set_var("XDG_CURRENT_DESKTOP", "KDE");
        }

        assert!(is_generic_wayland_without_supported_foreground_api());
        assert!(current_desktop_looks_like_gnome_or_kde());

        let mut provider = auto_foreground_provider();
        let snapshot = provider.sample(3_000);

        assert_eq!(provider.source(), ForegroundSource::Unsupported);
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unsupported);
        assert_eq!(snapshot.reason, GENERIC_WAYLAND_UNSUPPORTED_REASON);

        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
            restore_env_var("DISPLAY", previous_display);
            restore_env_var("XDG_CURRENT_DESKTOP", previous_desktop);
        }
    }

    #[test]
    fn sway_wayland_is_not_treated_as_generic_unsupported_wayland() {
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");

        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-1");
            std::env::set_var("SWAYSOCK", "/tmp/sway-ipc.sock");
            std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
        }

        assert!(!is_generic_wayland_without_supported_foreground_api());

        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
        }
    }

    #[test]
    fn hyprland_wayland_is_reserved_for_future_compositor_specific_provider() {
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");

        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-1");
            std::env::remove_var("SWAYSOCK");
            std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", "fake-hyprland-instance");
        }

        assert!(!is_generic_wayland_without_supported_foreground_api());

        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
        }
    }

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

    unsafe fn restore_env_var(name: &str, value: Option<std::ffi::OsString>) {
        if let Some(value) = value {
            unsafe { std::env::set_var(name, value) };
        } else {
            unsafe { std::env::remove_var(name) };
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
