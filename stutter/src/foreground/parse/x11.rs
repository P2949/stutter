//! X11 xprop parser helpers for foreground sampling.
//!
//! Owns X11 active-window and window-property parsing plus parser confidence scoring. Does not own
//! process execution or resolver stale-snapshot policy.

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct X11WindowProperties {
    pub(crate) pid: Option<u32>,
    pub(crate) instance: Option<String>,
    pub(crate) class: Option<String>,
    pub(crate) net_wm_name: Option<String>,
    pub(crate) wm_name: Option<String>,
}

pub(crate) fn parse_x11_active_window_id(output: &str) -> Option<String> {
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

pub(crate) fn parse_x11_window_properties(output: &str) -> X11WindowProperties {
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

pub(crate) fn parse_x11_quoted_strings(line: &str) -> Vec<String> {
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

pub(crate) fn x11_confidence(properties: &X11WindowProperties, window_id: &str) -> f32 {
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
