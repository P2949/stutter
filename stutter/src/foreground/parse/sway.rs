//! Sway tree parser helpers for foreground sampling.
//!
//! Owns Sway tree DTOs, focused-node lookup, and confidence scoring. Does not own process execution
//! or resolver stale-snapshot policy.

use serde::Deserialize;

use crate::foreground::model::{
    ForegroundProviderStatus, ForegroundSource, ForegroundWindowSnapshot,
};

#[derive(Debug, Deserialize)]
pub(crate) struct SwayNode {
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

pub(crate) fn focused_sway_snapshot_from_tree(
    elapsed_ms: u64,
    root: &SwayNode,
) -> ForegroundWindowSnapshot {
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

    if node.focused.unwrap_or(false) && is_sway_window_container(node) {
        return Some((node, current_workspace));
    }

    None
}

fn is_sway_window_container(node: &SwayNode) -> bool {
    node.node_type.as_deref() == Some("con")
        && (node.pid.is_some()
            || node
                .app_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || node.window.is_some()
            || node.window_properties.as_ref().is_some_and(|properties| {
                properties
                    .class
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    || properties
                        .instance
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
                    || properties
                        .title
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
            })
            || node
                .name
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()))
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
    } else if node.name.is_some() || node.window.is_some() {
        0.35
    } else {
        0.0
    }
}
