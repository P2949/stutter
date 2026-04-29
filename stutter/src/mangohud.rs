use std::{fs, path::Path};

use anyhow::Context;

use crate::recorder::FrameEvent;

pub fn read_frame_events(path: &Path) -> anyhow::Result<Vec<FrameEvent>> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read MangoHud log {}", path.display()))?;
    Ok(parse_frame_events(&data))
}

pub fn parse_frame_events(data: &str) -> Vec<FrameEvent> {
    let mut lines = data.lines().filter(|line| !line.trim().is_empty());
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let headers = split_csv_line(header);
    let elapsed_idx = find_header(&headers, &["elapsed_ms", "time_ms", "ms"]);
    let frametime_idx = find_header(&headers, &["frametime", "frametime_ms", "frame_time"]);

    lines
        .filter_map(|line| {
            let columns = split_csv_line(line);
            let frametime_ms = frametime_idx
                .and_then(|idx| columns.get(idx))
                .and_then(|value| value.parse::<f64>().ok())?;
            let elapsed_ms = elapsed_idx
                .and_then(|idx| columns.get(idx))
                .and_then(|value| value.parse::<f64>().ok())
                .map(|value| value.max(0.0) as u128)
                .unwrap_or(0);
            Some(FrameEvent {
                elapsed_ms,
                frametime_ms,
            })
        })
        .collect()
}

fn find_header(headers: &[String], candidates: &[&str]) -> Option<usize> {
    headers.iter().position(|header| {
        let normalized = header.trim().to_ascii_lowercase();
        candidates.iter().any(|candidate| normalized == *candidate)
    })
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                values.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    values.push(current.trim().to_owned());
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_based_frametime_csv() {
        let events = parse_frame_events("elapsed_ms,frametime_ms\n10,16.7\n20,33.4\n");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].elapsed_ms, 10);
        assert_eq!(events[1].frametime_ms, 33.4);
    }
}
