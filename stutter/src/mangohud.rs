use std::{fs, io::BufRead, path::Path};

use anyhow::Context;

use crate::recorder::FrameEvent;

pub fn read_frame_events(path: &Path) -> anyhow::Result<Vec<FrameEvent>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to read MangoHud log {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    parse_frame_events(reader.lines())
}

pub fn parse_frame_events<I>(mut lines: I) -> anyhow::Result<Vec<FrameEvent>>
where
    I: Iterator<Item = std::io::Result<String>>,
{
    let Some(header) = lines.next() else {
        return Ok(Vec::new());
    };
    let header = header?;
    if header.trim().is_empty() {
        return Ok(Vec::new());
    }
    let headers = split_csv_line(&header);
    let elapsed_idx = find_header(&headers, &["elapsed_ms", "time_ms", "ms"]);
    let frametime_idx = find_header(&headers, &["frametime", "frametime_ms", "frame_time"]);

    let mut events = Vec::new();
    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let columns = split_csv_line(&line);
        if let Some(frametime_ms) = frametime_idx
            .and_then(|idx| columns.get(idx))
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
        {
            let elapsed_ms = elapsed_idx
                .and_then(|idx| columns.get(idx))
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite())
                .map(|value| value.max(0.0) as u128)
                .unwrap_or(0);
            events.push(FrameEvent {
                elapsed_ms,
                frametime_ms,
            });
        }
    }

    Ok(events)
}

fn find_header(headers: &[String], candidates: &[&str]) -> Option<usize> {
    headers.iter().position(|header| {
        let normalized = header.trim().to_ascii_lowercase();
        candidates.iter().any(|candidate| normalized == *candidate)
    })
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(field.trim().to_owned());
                field.clear();
            }
            _ => field.push(ch),
        }
    }

    fields.push(field.trim().to_owned());
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_based_frametime_csv() {
        let data = "elapsed_ms,frametime_ms\n10,16.7\n20,33.4\n";
        let events = parse_frame_events(data.lines().map(|s| Ok(s.to_owned()))).unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].elapsed_ms, 10);
        assert_eq!(events[1].frametime_ms, 33.4);
    }

    #[test]
    fn parses_quoted_csv_fields() {
        let data = "elapsed_ms,\"frame,time\",frametime_ms\n10,\"ignored, value\",16.7\n";
        let events = parse_frame_events(data.lines().map(|s| Ok(s.to_owned()))).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].elapsed_ms, 10);
        assert_eq!(events[0].frametime_ms, 16.7);
    }

    #[test]
    fn skips_non_finite_frametimes() {
        let data = "elapsed_ms,frametime_ms\n10,NaN\n20,inf\n30,16.7\n";
        let events = parse_frame_events(data.lines().map(|s| Ok(s.to_owned()))).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].elapsed_ms, 30);
        assert_eq!(events[0].frametime_ms, 16.7);
    }
}
